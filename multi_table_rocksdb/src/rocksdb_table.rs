use std::sync::Arc;
use std::fmt;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::thread;
use std::sync::mpsc;

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

use laoflchdb_engines::{StorageEngine, ColumnFilter, ColumnFilterCondition, FilterOperator};

use crate::MultiTableRocksDBEngine;

#[derive(Debug)]
pub struct RocksDBTable {
    engine: Arc<TokioRwLock<MultiTableRocksDBEngine>>,
    table_name: String,
    schema: Arc<Schema>,
}

impl RocksDBTable {
    pub async fn new(engine: Arc<TokioRwLock<MultiTableRocksDBEngine>>, table_name: &str) -> Self {
        let schema = {
            let engine_guard = engine.read().await;
            match StorageEngine::list_table_cols(&*engine_guard, table_name).await {
                Ok(columns) => {
                    let arrow_fields: Vec<datafusion::arrow::datatypes::Field> = columns.into_iter()
                        .map(|col| {
                            let col_type = col.column_type.enum_value_or_default();
                            let data_type = engine_guard.column_type_to_arrow_type(&col_type);
                            ArrowField::new(&col.column_name, data_type, true)
                        })
                        .collect();
                    Arc::new(Schema::new(arrow_fields))
                }
                Err(_) => Arc::new(Schema::new(Vec::<datafusion::arrow::datatypes::Field>::new())),
            }
        };
        Self {
            engine,
            table_name: table_name.to_string(),
            schema,
        }
    }
    
    fn parse_filters(&self, filters: &[Expr]) -> Vec<ColumnFilter> {
        let mut column_filters = Vec::new();
        
        for filter in filters {
            match filter {
                Expr::BinaryExpr(BinaryExpr { left, op, right }) => {
                    if let Expr::Column(c) = left.as_ref() {
                        let column_name = c.name.clone();
                        
                        let pb_field = match right.as_ref() {
                            Expr::Literal(lit, _) => {
                                use datafusion::scalar::ScalarValue;
                                let mut field = laoflchdb_engines::Field::new();
                                match lit {
                                    ScalarValue::Int64(Some(v)) => {
                                        field.value = Some(laoflchdb_engines::field::field::Value::IntegerValue(laoflchdb_engines::field::Integer {
                                            value: *v,
                                            special_fields: ::protobuf::SpecialFields::default(),
                                        }));
                                    }
                                    ScalarValue::Float64(Some(v)) => {
                                        field.value = Some(laoflchdb_engines::field::field::Value::FloatValue(laoflchdb_engines::field::Float {
                                            value: *v,
                                            special_fields: ::protobuf::SpecialFields::default(),
                                        }));
                                    }
                                    ScalarValue::Utf8(Some(v)) | ScalarValue::LargeUtf8(Some(v)) => {
                                        field.value = Some(laoflchdb_engines::field::field::Value::StringValue(laoflchdb_engines::field::String {
                                            value: v.clone(),
                                            special_fields: ::protobuf::SpecialFields::default(),
                                        }));
                                    }
                                    ScalarValue::Binary(Some(v)) | ScalarValue::LargeBinary(Some(v)) => {
                                        field.value = Some(laoflchdb_engines::field::field::Value::BytesValue(laoflchdb_engines::field::Bytes {
                                            value: v.clone(),
                                            special_fields: ::protobuf::SpecialFields::default(),
                                        }));
                                    }
                                    _ => {
                                        field.value = Some(laoflchdb_engines::field::field::Value::StringValue(laoflchdb_engines::field::String {
                                            value: lit.to_string(),
                                            special_fields: ::protobuf::SpecialFields::default(),
                                        }));
                                    }
                                }
                                field
                            }
                            _ => {
                                let mut field = laoflchdb_engines::Field::new();
                                field.value = Some(laoflchdb_engines::field::field::Value::StringValue(laoflchdb_engines::field::String {
                                    value: right.to_string(),
                                    special_fields: ::protobuf::SpecialFields::default(),
                                }));
                                field
                            }
                        };
                        
                        let filter_op = match op {
                            Operator::Eq => FilterOperator::FILTER_OPERATOR_EQ,
                            Operator::NotEq => FilterOperator::FILTER_OPERATOR_NEQ,
                            Operator::Lt => FilterOperator::FILTER_OPERATOR_LT,
                            Operator::Gt => FilterOperator::FILTER_OPERATOR_GT,
                            Operator::LtEq => FilterOperator::FILTER_OPERATOR_LTE,
                            Operator::GtEq => FilterOperator::FILTER_OPERATOR_GTE,
                            _ => continue,
                        };
                        
                        column_filters.push(ColumnFilter {
                            column_name,
                            conditions: vec![ColumnFilterCondition {
                                op: filter_op.into(),
                                value: Some(pb_field).into(),
                                values: Vec::new(),
                                special_fields: ::protobuf::SpecialFields::default(),
                            }],
                            special_fields: ::protobuf::SpecialFields::default(),
                        });
                    }
                }
                _ => {}
            }
        }
        
        column_filters
    }
}

#[derive(Debug, Clone)]
struct RocksScanExec {
    engine: Arc<TokioRwLock<MultiTableRocksDBEngine>>,
    table_name: String,
    projection: Option<Vec<usize>>,
    filters: Vec<ColumnFilter>,
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
        let filters = self.filters.clone();
        let limit = self.limit;
        
        let (tx, rx) = mpsc::channel();
        
        thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            let result = rt.block_on(async {
                let engine_guard = engine.read().await;
                engine_guard.table_to_arrow_with_pushdown(
                    &table_name,
                    projection.as_ref(),
                    &filters,
                    limit
                ).await
            });
            
            let _ = tx.send(result);
        });
        
        let result = rx.recv().map_err(|e| {
            datafusion::error::DataFusionError::Execution(format!("Thread communication error: {}", e))
        })?;
        
        let (_, arrays, _) = result.map_err(|e| {
            datafusion::error::DataFusionError::Execution(e.to_string())
        })?;
        
        let batch = RecordBatch::try_new(self.schema.clone(), arrays)?;
        
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
        
        /// Filter Pushdown 类型说明:
        /// 
        /// - `Exact`: 过滤器可以精确下推到存储层执行，返回的结果与在内存中过滤完全一致
        ///   支持的比较操作符: =, !=, <, >, <=, >=, BETWEEN, IN, LIKE, IS NULL, IS NOT NULL
        /// 
        /// - `Inexact`: 过滤器可以下推，但结果可能不完全精确
        ///   例如：使用了存储层不完全支持的函数（如正则表达式）
        /// 
        /// - `Unsupported`: 过滤器不能下推，必须在内存中执行
        ///   例如：使用了存储层不支持的函数或表达式
        
        let mut supported = Vec::new();
        for filter in filters {
            match filter {
                datafusion::logical_expr::Expr::BinaryExpr(datafusion::logical_expr::BinaryExpr { op, .. }) => match op {
                    // 支持的比较操作符可以精确下推
                    datafusion::logical_expr::Operator::Eq |       // = (等于)
                    datafusion::logical_expr::Operator::NotEq |    // != (不等于)
                    datafusion::logical_expr::Operator::Lt |       // < (小于)
                    datafusion::logical_expr::Operator::Gt |       // > (大于)
                    datafusion::logical_expr::Operator::LtEq |    // <= (小于等于)
                    datafusion::logical_expr::Operator::GtEq => {  // >= (大于等于)
                        supported.push(TableProviderFilterPushDown::Exact)
                    }
                    _ => supported.push(TableProviderFilterPushDown::Unsupported),
                },
                // 其他表达式类型（如函数调用、LIKE等）暂不支持下推
                _ => supported.push(TableProviderFilterPushDown::Unsupported),
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
        let column_filters = self.parse_filters(filters);
        
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
            filters: column_filters,
            limit,
            schema: projected_schema,
            properties,
        }))
    }
}
