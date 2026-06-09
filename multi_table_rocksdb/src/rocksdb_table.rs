use std::sync::Arc;
use std::fmt;
use std::pin::Pin;
use std::task::{Context, Poll};

use datafusion::arrow::datatypes::{Field as ArrowField, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::catalog::Session;
use datafusion::datasource::TableProvider;
use datafusion::execution::context::TaskContext;
use datafusion::logical_expr::{BinaryExpr, Expr, Operator};
use datafusion::physical_plan::{ExecutionPlan, DisplayAs, Partitioning, PlanProperties, RecordBatchStream, SendableRecordBatchStream};
use datafusion::physical_plan::execution_plan::{Boundedness, EmissionType, SchedulingType};
use datafusion_physical_expr::EquivalenceProperties;
use futures::Stream;
use tokio::sync::RwLock as TokioRwLock;

use laoflchdb_engines::{StorageEngine, ColumnFilter, ColumnFilterCondition, FilterOperator, Row};

/// 过滤器项
#[derive(Debug, Clone)]
pub enum FilterItem {
    /// 列过滤器
    ColumnFilter(ColumnFilter),
    /// 嵌套过滤器组
    Group(FilterGroup),
}

/// 过滤器组：支持任意 AND/OR 嵌套结构
#[derive(Debug, Clone)]
pub struct FilterGroup {
    /// 组内关系
    pub relation: FilterRelation,
    /// 组内项
    pub items: Vec<FilterItem>,
}

/// 逻辑关系
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterRelation {
    And,
    Or,
}

/// 检查过滤器组是否为空
impl FilterGroup {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl Default for FilterGroup {
    fn default() -> Self {
        Self {
            relation: FilterRelation::And,
            items: Vec::new(),
        }
    }
}

use crate::MultiTableRocksDBEngine;

#[derive(Debug)]
pub struct RocksDBTable {
    engine: Arc<MultiTableRocksDBEngine>,
    table_name: String,
    schema: Arc<Schema>,
}

impl RocksDBTable {
    /// 从带 schema 前缀的表名中提取纯表名
    /// 例如: "example.orders" -> "orders"
    ///       "orders" -> "orders"
    fn extract_table_name(full_name: &str) -> &str {
        if let Some(pos) = full_name.rfind('.') {
            &full_name[pos + 1..]
        } else {
            full_name
        }
    }

    pub async fn new(engine: Arc<MultiTableRocksDBEngine>, table_name: &str) -> Self {
        // 提取纯表名（去掉 schema 前缀）
        let raw_table_name = Self::extract_table_name(table_name);
        log::info!("[RocksDBTable] Creating table provider for '{}' (raw: '{}')", table_name, raw_table_name);
        
        let schema = match StorageEngine::list_table_cols(&*engine, raw_table_name).await {
            Ok(columns) => {
                log::info!("[RocksDBTable] Found {} columns for table '{}': {:?}", 
                    columns.len(), raw_table_name, columns.iter().map(|c| c.column_name.clone()).collect::<Vec<_>>());
                let arrow_fields: Vec<datafusion::arrow::datatypes::Field> = columns.into_iter()
                    .map(|col| {
                        let col_type = col.column_type.enum_value_or_default();
                        let data_type = engine.column_type_to_arrow_type(&col_type);
                        ArrowField::new(&col.column_name, data_type, true)
                    })
                    .collect();
                Arc::new(Schema::new(arrow_fields))
            }
            Err(e) => {
                log::warn!("[RocksDBTable] Failed to get columns for table '{}': {}", raw_table_name, e);
                Arc::new(Schema::new(Vec::<datafusion::arrow::datatypes::Field>::new()))
            }
        };
        Self {
            engine,
            table_name: raw_table_name.to_string(),
            schema,
        }
    }
    
    /// 解析过滤器表达式，构建 FilterGroup 树
    /// 
    /// 返回：(FilterGroup, 是否需要对结果取反)
    fn parse_filters(&self, filters: &[Expr]) -> (FilterGroup, bool) {
        let mut root = FilterGroup {
            relation: FilterRelation::And,
            items: Vec::new(),
        };
        let mut negate_result = false;
        
        for filter in filters {
            let (item, needs_negate) = self.parse_filter_expr(filter, false);
            if let Some(item) = item {
                root.items.push(item);
            }
            if needs_negate {
                negate_result = !negate_result;
            }
        }
        
        (root, negate_result)
    }
    
    /// 解析单个表达式，返回 FilterItem
    fn parse_filter_expr(&self, expr: &Expr, negate: bool) -> (Option<FilterItem>, bool) {
        match expr {
            Expr::BinaryExpr(BinaryExpr { left, op, right }) => {
                match op {
                    Operator::And => {
                        if negate {
                            // NOT (A AND B) = NOT A OR NOT B
                            let (left_item, _) = self.parse_filter_expr(left, true);
                            let (right_item, _) = self.parse_filter_expr(right, true);
                            
                            // 构建 OR 组
                            let mut items = Vec::new();
                            if let Some(item) = left_item {
                                items.push(item);
                            }
                            if let Some(item) = right_item {
                                items.push(item);
                            }
                            
                            if items.is_empty() {
                                return (None, false);
                            }
                            
                            let group = FilterGroup {
                                relation: FilterRelation::Or,
                                items,
                            };
                            return (Some(FilterItem::Group(group)), true);
                        } else {
                            // A AND B：构建 AND 组
                            let (left_item, _) = self.parse_filter_expr(left, false);
                            let (right_item, _) = self.parse_filter_expr(right, false);
                            
                            let mut items = Vec::new();
                            if let Some(item) = left_item {
                                items.push(item);
                            }
                            if let Some(item) = right_item {
                                items.push(item);
                            }
                            
                            if items.is_empty() {
                                return (None, false);
                            }
                            
                            if items.len() == 1 {
                                return (Some(items.remove(0)), false);
                            }
                            
                            let group = FilterGroup {
                                relation: FilterRelation::And,
                                items,
                            };
                            return (Some(FilterItem::Group(group)), false);
                        }
                    }
                    Operator::Or => {
                        if negate {
                            // NOT (A OR B) = NOT A AND NOT B
                            let (left_item, _) = self.parse_filter_expr(left, true);
                            let (right_item, _) = self.parse_filter_expr(right, true);
                            
                            let mut items = Vec::new();
                            if let Some(item) = left_item {
                                items.push(item);
                            }
                            if let Some(item) = right_item {
                                items.push(item);
                            }
                            
                            if items.is_empty() {
                                return (None, false);
                            }
                            
                            if items.len() == 1 {
                                return (Some(items.remove(0)), false);
                            }
                            
                            let group = FilterGroup {
                                relation: FilterRelation::And,
                                items,
                            };
                            return (Some(FilterItem::Group(group)), false);
                        } else {
                            // A OR B：构建 OR 组
                            let (left_item, _) = self.parse_filter_expr(left, false);
                            let (right_item, _) = self.parse_filter_expr(right, false);
                            
                            let mut items = Vec::new();
                            if let Some(item) = left_item {
                                items.push(item);
                            }
                            if let Some(item) = right_item {
                                items.push(item);
                            }
                            
                            if items.is_empty() {
                                return (None, false);
                            }
                            
                            if items.len() == 1 {
                                return (Some(items.remove(0)), false);
                            }
                            
                            let group = FilterGroup {
                                relation: FilterRelation::Or,
                                items,
                            };
                            return (Some(FilterItem::Group(group)), false);
                        }
                    }
                    _ => {}
                }
                
                // 处理比较操作符
                if let Some(filter) = self.handle_comparison_op(left, op, right, negate) {
                    return (Some(FilterItem::ColumnFilter(filter)), false);
                }
            }
            // 处理 NOT 表达式
            Expr::Not(inner) => {
                return self.parse_filter_expr(inner, !negate);
            }
            _ => {}
        }
        
        (None, false)
    }
    
    /// 处理比较操作符，negate 为 true 时使用反向操作符
    fn handle_comparison_op(
        &self,
        left: &Box<Expr>,
        op: &Operator,
        right: &Box<Expr>,
        negate: bool,
    ) -> Option<ColumnFilter> {
        // 获取左边的列名
        let column_name = match left.as_ref() {
            Expr::Column(c) => c.name.clone(),
            _ => return None,
        };
        
        let pb_field = match right.as_ref() {
            Expr::Literal(lit, _) => {
                self.literal_to_field(lit)
            }
            _ => {
                let mut field = laoflchdb_engines::Field::new();
                field.value = Some(laoflchdb_engines::field::field::Value::StringValue(
                    laoflchdb_engines::field::String {
                        value: right.to_string(),
                        special_fields: ::protobuf::SpecialFields::default(),
                    }
                ));
                field
            }
        };
        
        // 根据是否取反，选择对应的操作符
        let filter_op = match (op, negate) {
            // negate = false: 原始操作符
            (Operator::Eq, false) => FilterOperator::FILTER_OPERATOR_EQ,
            (Operator::NotEq, false) => FilterOperator::FILTER_OPERATOR_NEQ,
            (Operator::Lt, false) => FilterOperator::FILTER_OPERATOR_LT,
            (Operator::Gt, false) => FilterOperator::FILTER_OPERATOR_GT,
            (Operator::LtEq, false) => FilterOperator::FILTER_OPERATOR_LTE,
            (Operator::GtEq, false) => FilterOperator::FILTER_OPERATOR_GTE,
            // negate = true: 反向操作符 (德摩根定律)
            (Operator::Eq, true) => FilterOperator::FILTER_OPERATOR_NEQ,  // NOT (a = b) = a != b
            (Operator::NotEq, true) => FilterOperator::FILTER_OPERATOR_EQ, // NOT (a != b) = a = b
            (Operator::Lt, true) => FilterOperator::FILTER_OPERATOR_GTE,  // NOT (a < b) = a >= b
            (Operator::Gt, true) => FilterOperator::FILTER_OPERATOR_LTE,  // NOT (a > b) = a <= b
            (Operator::LtEq, true) => FilterOperator::FILTER_OPERATOR_GT, // NOT (a <= b) = a > b
            (Operator::GtEq, true) => FilterOperator::FILTER_OPERATOR_LT, // NOT (a >= b) = a < b
            _ => return None,
        };
        
        Some(ColumnFilter {
            column_name,
            conditions: vec![ColumnFilterCondition {
                op: filter_op.into(),
                value: Some(pb_field).into(),
                values: Vec::new(),
                special_fields: ::protobuf::SpecialFields::default(),
            }],
            special_fields: ::protobuf::SpecialFields::default(),
        })
    }
    
    /// 将 ScalarValue 转换为 Field
    fn literal_to_field(&self, lit: &datafusion::scalar::ScalarValue) -> laoflchdb_engines::Field {
        use datafusion::scalar::ScalarValue;
        let mut field = laoflchdb_engines::Field::new();
        
        match lit {
            ScalarValue::Int64(Some(v)) => {
                field.value = Some(laoflchdb_engines::field::field::Value::IntegerValue(
                    laoflchdb_engines::field::Integer {
                        value: *v,
                        special_fields: ::protobuf::SpecialFields::default(),
                    }
                ));
            }
            ScalarValue::Float64(Some(v)) => {
                field.value = Some(laoflchdb_engines::field::field::Value::FloatValue(
                    laoflchdb_engines::field::Float {
                        value: *v,
                        special_fields: ::protobuf::SpecialFields::default(),
                    }
                ));
            }
            ScalarValue::Utf8(Some(v)) | ScalarValue::LargeUtf8(Some(v)) => {
                field.value = Some(laoflchdb_engines::field::field::Value::StringValue(
                    laoflchdb_engines::field::String {
                        value: v.clone(),
                        special_fields: ::protobuf::SpecialFields::default(),
                    }
                ));
            }
            ScalarValue::Binary(Some(v)) | ScalarValue::LargeBinary(Some(v)) => {
                field.value = Some(laoflchdb_engines::field::field::Value::BytesValue(
                    laoflchdb_engines::field::Bytes {
                        value: v.clone(),
                        special_fields: ::protobuf::SpecialFields::default(),
                    }
                ));
            }
            _ => {
                field.value = Some(laoflchdb_engines::field::field::Value::StringValue(
                    laoflchdb_engines::field::String {
                        value: lit.to_string(),
                        special_fields: ::protobuf::SpecialFields::default(),
                    }
                ));
            }
        }
        field
    }
    
    /// 添加列过滤器
    fn add_column_filter(
        &self,
        column_filters: &mut Vec<ColumnFilter>,
        column_name: String,
        op: FilterOperator,
        field: laoflchdb_engines::Field,
    ) {
        // 检查是否已经有该列的过滤器，如果有则添加条件
        if let Some(existing_filter) = column_filters.iter_mut().find(|cf| cf.column_name == column_name) {
            existing_filter.conditions.push(ColumnFilterCondition {
                op: op.into(),
                value: Some(field).into(),
                values: Vec::new(),
                special_fields: ::protobuf::SpecialFields::default(),
            });
        } else {
            column_filters.push(ColumnFilter {
                column_name,
                conditions: vec![ColumnFilterCondition {
                    op: op.into(),
                    value: Some(field).into(),
                    values: Vec::new(),
                    special_fields: ::protobuf::SpecialFields::default(),
                }],
                special_fields: ::protobuf::SpecialFields::default(),
            });
        }
    }
}

#[derive(Debug, Clone)]
struct RocksScanExec {
    engine: Arc<MultiTableRocksDBEngine>,
    table_name: String,
    projection: Option<Vec<usize>>,
    filter_group: FilterGroup,        // 使用 FilterGroup 支持任意 AND/OR 嵌套
    negate_result: bool,              // 是否对结果取反
    limit: Option<usize>,
    schema: Arc<Schema>,
    properties: Arc<PlanProperties>,
}

impl DisplayAs for RocksScanExec {
    fn fmt_as(&self, _t: datafusion::physical_plan::DisplayFormatType, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "RocksScanExec: table={}", self.table_name)
    }
}

impl ExecutionPlan for RocksScanExec {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    
    fn schema(&self) -> Arc<Schema> {
        self.schema.clone()
    }
    
    fn name(&self) -> &str {
        "RocksScanExec"
    }
    
    fn properties(&self) -> &Arc<PlanProperties> {
        &self.properties
    }
    
    fn children(&self) -> Vec<&Arc<dyn ExecutionPlan>> {
        Vec::new()
    }
    
    fn with_new_children(
        self: Arc<Self>,
        _children: Vec<Arc<dyn ExecutionPlan>>,
    ) -> datafusion::error::Result<Arc<dyn ExecutionPlan>> {
        Ok(self)
    }
    
    fn execute(
        &self,
        _partition: usize,
        _context: Arc<TaskContext>,
    ) -> datafusion::error::Result<SendableRecordBatchStream> {
        let engine = self.engine.clone();
        let table_name = self.table_name.clone();
        let projection = self.projection.clone();
        let filter_group = self.filter_group.clone();
        let negate_result = self.negate_result;
        let limit = self.limit;
        let schema = self.schema.clone();
        
        // 直接执行同步操作，因为 DataFusion 会在适当的上下文中调用 execute
        let result = engine.table_to_arrow_with_filter_group_sync(
            &table_name,
            projection.as_ref(),
            &filter_group,
            limit,
            negate_result
        ).map_err(|e| {
            datafusion::error::DataFusionError::Execution(e.to_string())
        })?;
        
        let (_, arrays, _) = result;
        
        // 检查是否有数据，如果没有则返回空的 batch
        if arrays.is_empty() || arrays[0].len() == 0 {
            let empty_batch = RecordBatch::new_empty(schema);
            return Ok(Box::pin(RocksBatchStream::new(vec![empty_batch])));
        }
        
        let batch = RecordBatch::try_new(schema, arrays)?;
        
        Ok(Box::pin(RocksBatchStream::new(vec![batch])))
    }
}

struct RocksBatchStream {
    batches: Vec<RecordBatch>,
    index: usize,
}

impl RocksBatchStream {
    fn new(batches: Vec<RecordBatch>) -> Self {
        Self { batches, index: 0 }
    }
}

impl Stream for RocksBatchStream {
    type Item = datafusion::error::Result<RecordBatch>;
    
    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.index < self.batches.len() {
            let batch = self.batches[self.index].clone();
            self.index += 1;
            Poll::Ready(Some(Ok(batch)))
        } else {
            Poll::Ready(None)
        }
    }
}

impl RecordBatchStream for RocksBatchStream {
    fn schema(&self) -> Arc<Schema> {
        if let Some(batch) = self.batches.first() {
            batch.schema()
        } else {
            let empty_fields: Vec<datafusion::arrow::datatypes::Field> = Vec::new();
            Arc::new(Schema::new(empty_fields))
        }
    }
}

#[::async_trait::async_trait]
impl TableProvider for RocksDBTable {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    
    fn schema(&self) -> Arc<Schema> {
        self.schema.clone()
    }
    
    fn table_type(&self) -> datafusion::datasource::TableType {
        datafusion::datasource::TableType::Base
    }
    
    fn supports_filters_pushdown(
        &self,
        filters: &[&datafusion::logical_expr::Expr],
    ) -> datafusion::error::Result<Vec<datafusion_expr::TableProviderFilterPushDown>> {
        use datafusion_expr::TableProviderFilterPushDown;
        
        /// 判断单个表达式是否支持下推
        fn is_supported(expr: &datafusion::logical_expr::Expr) -> bool {
            match expr {
                datafusion::logical_expr::Expr::BinaryExpr(datafusion::logical_expr::BinaryExpr { op, left, right }) => match op {
                    // AND 表达式：两个子表达式都支持才支持
                    datafusion::logical_expr::Operator::And => {
                        is_supported(left) && is_supported(right)
                    },
                    // OR 表达式：两个子表达式都支持才支持
                    datafusion::logical_expr::Operator::Or => {
                        is_supported(left) && is_supported(right)
                    },
                    // 支持的比较操作符可以精确下推
                    datafusion::logical_expr::Operator::Eq |       // = (等于)
                    datafusion::logical_expr::Operator::NotEq |    // != (不等于)
                    datafusion::logical_expr::Operator::Lt |       // < (小于)
                    datafusion::logical_expr::Operator::Gt |       // > (大于)
                    datafusion::logical_expr::Operator::LtEq |    // <= (小于等于)
                    datafusion::logical_expr::Operator::GtEq => true,  // >= (大于等于)
                    _ => false,
                },
                // 其他表达式类型暂不支持
                _ => false,
            }
        }
        
        /// Filter Pushdown 类型说明:
        /// 
        /// - `Exact`: 过滤器可以精确下推到存储层执行，返回的结果与在内存中过滤完全一致
        ///   支持的比较操作符: =, !=, <, >, <=, >=
        ///   支持的逻辑操作符: AND, OR (仅当所有子表达式都支持时)
        /// 
        /// - `Inexact`: 过滤器可以下推，但结果可能不完全精确
        ///   例如：使用了存储层不完全支持的函数
        /// 
        /// - `Unsupported`: 过滤器不能下推，必须在内存中执行
        ///   例如：使用了存储层不支持的函数或表达式
        
        let mut supported = Vec::new();
        for filter in filters {
            if is_supported(filter) {
                supported.push(TableProviderFilterPushDown::Exact);
            } else {
                supported.push(TableProviderFilterPushDown::Unsupported);
            }
        }
        Ok(supported)
    }
    
    async fn scan(
        &self,
        _ctx: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[datafusion::logical_expr::Expr],
        limit: Option<usize>,
    ) -> datafusion::error::Result<Arc<dyn ExecutionPlan>> {
        let (filter_group, negate_result) = self.parse_filters(filters);
        
        let projected_schema = match projection {
            Some(p) => {
                let fields: Vec<_> = p.iter()
                    .filter(|&&idx| idx < self.schema.fields().len())
                    .map(|&idx| self.schema.field(idx).clone())
                    .collect();
                Arc::new(Schema::new(fields))
            }
            None => self.schema.clone(),
        };
        
        let properties = Arc::new(PlanProperties::new(
            EquivalenceProperties::new(projected_schema.clone()),
            Partitioning::UnknownPartitioning(1),
            EmissionType::Incremental,
            Boundedness::Bounded,
        ).with_scheduling_type(SchedulingType::NonCooperative));
        
        Ok(Arc::new(RocksScanExec {
            engine: self.engine.clone(),
            table_name: self.table_name.clone(),
            projection: projection.cloned(),
            filter_group,
            negate_result,
            limit,
            schema: projected_schema,
            properties,
        }))
    }
}
