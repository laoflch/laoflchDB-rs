use std::sync::Arc;

use datafusion::arrow::array::{StringArray, Int64Array, Float64Array, BinaryArray};
use datafusion::arrow::datatypes::{DataType, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::datasource::TableProvider;
use datafusion::execution::context::{SessionContext, SessionConfig};
use datafusion::sql::TableReference;
use datafusion::common::TableReference as CommonTableReference;
use datafusion_catalog::CatalogProvider;
use protobuf::Message;

use laoflchdb_engines::{StorageEngine, SQLEngine, QueryResult, QueryRow};
use laoflchdb_engines::field::Field as PbField;
use laoflchdb_engines::field::field::Value;

#[async_trait::async_trait]
pub trait DataFusionStorageEngine: Send + Sync + 'static {
    async fn table_to_arrow(&self, table_name: &str) -> Result<(Schema, Vec<datafusion::arrow::array::ArrayRef>, Vec<(i32, String)>), Box<dyn std::error::Error + Send + Sync>>;
}

pub struct DataFusionSQLEngine<E: StorageEngine + DataFusionStorageEngine> {
    storage_engine: Arc<tokio::sync::RwLock<E>>,
    ctx: SessionContext,
}



impl<E: StorageEngine + DataFusionStorageEngine> DataFusionSQLEngine<E> {
    pub fn new(storage_engine: Arc<tokio::sync::RwLock<E>>) -> Self {
        let mut config = SessionConfig::new();
        config = config.with_default_catalog_and_schema("laoflchdb", "sys");
        
        let ctx = SessionContext::new_with_config(config);
        
        use datafusion_catalog::memory::{MemoryCatalogProvider, MemorySchemaProvider};
        
        let catalog = Arc::new(MemoryCatalogProvider::new());
        let schema = Arc::new(MemorySchemaProvider::new());
        let _ = catalog.register_schema("sys", schema);
        let _ = ctx.register_catalog("laoflchdb", catalog);
        
        Self {
            storage_engine,
            ctx,
        }
    }
    
    fn arrow_to_query_result(&self, schema: &Schema, batch: &RecordBatch, _column_infos: &[(i32, String)]) -> QueryResult {
        let mut proto_rows = Vec::new();
        
        for i in 0..batch.num_rows() {
            let mut row_data = Vec::new();
            
            for (j, field) in schema.fields().iter().enumerate() {
                let array = batch.column(j);
                
                let pb_field = match field.data_type() {
                    DataType::Utf8 => {
                        let array = array.as_any().downcast_ref::<StringArray>().unwrap();
                        let value = array.value(i);
                        PbField {
                            value: Some(Value::StringValue(laoflchdb_engines::field::String {
                                value: value.to_string(),
                                special_fields: ::protobuf::SpecialFields::default(),
                            })),
                            special_fields: ::protobuf::SpecialFields::default(),
                        }
                    }
                    DataType::Int64 => {
                        let array = array.as_any().downcast_ref::<Int64Array>().unwrap();
                        let value = array.value(i);
                        PbField {
                            value: Some(Value::IntegerValue(laoflchdb_engines::field::Integer {
                                value,
                                special_fields: ::protobuf::SpecialFields::default(),
                            })),
                            special_fields: ::protobuf::SpecialFields::default(),
                        }
                    }
                    DataType::Float64 => {
                        let array = array.as_any().downcast_ref::<Float64Array>().unwrap();
                        let value = array.value(i);
                        PbField {
                            value: Some(Value::FloatValue(laoflchdb_engines::field::Float {
                                value,
                                special_fields: ::protobuf::SpecialFields::default(),
                            })),
                            special_fields: ::protobuf::SpecialFields::default(),
                        }
                    }
                    DataType::Binary => {
                        let array = array.as_any().downcast_ref::<BinaryArray>().unwrap();
                        let value = array.value(i);
                        PbField {
                            value: Some(Value::BytesValue(laoflchdb_engines::field::Bytes {
                                value: value.to_vec(),
                                special_fields: ::protobuf::SpecialFields::default(),
                            })),
                            special_fields: ::protobuf::SpecialFields::default(),
                        }
                    }
                    _ => PbField::default(),
                };
                
                let value_bytes = pb_field.write_to_bytes().unwrap_or_default();
                row_data.push(value_bytes);
            }
            
            proto_rows.push(row_data);
        }
        
        let mut query_rows = Vec::new();
        for (idx, data) in proto_rows.into_iter().enumerate() {
            let row = laoflchdb_engines::row::Row {
                row_type: protobuf::EnumOrUnknown::default(),
                version: 0,
                data,
                special_fields: ::protobuf::SpecialFields::default(),
            };
            
            query_rows.push(QueryRow {
                table_name: "".to_string(),
                row_id: idx as u64,
                row: ::protobuf::MessageField::some(row),
                special_fields: ::protobuf::SpecialFields::default(),
            });
        }
        
        let columns: Vec<String> = schema.fields().iter()
            .map(|f| f.name().to_string())
            .collect();
        
        QueryResult {
            rows: query_rows,
            columns,
            special_fields: ::protobuf::SpecialFields::default(),
        }
    }
}

#[async_trait::async_trait]
impl<E: StorageEngine + DataFusionStorageEngine + 'static> SQLEngine for DataFusionSQLEngine<E> {
    async fn execute_query(&self, sql: &str) -> Result<QueryResult, Box<dyn std::error::Error + Send + Sync>> {
        use std::time::Instant;
        
        let start_total = Instant::now();
        log::info!("[SQL] 开始执行查询: {}", sql);
        
        let ctx = self.ctx.clone();
        let sql = sql.to_string();
        
        // 步骤1: SQL 解析
        let start = Instant::now();
        let df = ctx.sql(sql.as_str()).await.map_err(|e| {
            log::error!("[SQL] SQL 解析失败: {}", e);
            Box::new(e) as Box<dyn std::error::Error + Send + Sync>
        })?;
        log::info!("[SQL] 步骤1 - SQL 解析完成，耗时: {:?}", start.elapsed());
        
        // 步骤2: 生成逻辑计划
        let start = Instant::now();
        let logical_plan = df.logical_plan();
        log::info!("[SQL] 步骤2 - 逻辑计划生成完成，耗时: {:?}", start.elapsed());
        log::debug!("[SQL] 逻辑计划:\n{}", logical_plan);
        
        // 步骤3: 生成物理计划
        let start = Instant::now();
        let physical_plan = df.clone().create_physical_plan().await.map_err(|e| {
            log::error!("[SQL] 物理计划生成失败: {}", e);
            Box::new(e) as Box<dyn std::error::Error + Send + Sync>
        })?;
        log::info!("[SQL] 步骤3 - 物理计划生成完成，耗时: {:?}", start.elapsed());
        log::debug!("[SQL] 物理计划:\n{:?}", physical_plan);
        
        // 步骤4: 执行查询
        let start = Instant::now();
        log::info!("[SQL] 步骤4 - 开始执行查询...");
        let batches = df.collect().await.map_err(|e| {
            log::error!("[SQL] 查询执行失败: {}", e);
            Box::new(e) as Box<dyn std::error::Error + Send + Sync>
        })?;
        log::info!("[SQL] 步骤4 - 查询执行完成，耗时: {:?}, 返回 {} 个批次", start.elapsed(), batches.len());
        
        // 步骤5: 结果转换
        let start = Instant::now();
        if batches.is_empty() {
            log::info!("[SQL] 查询结果为空，总耗时: {:?}", start_total.elapsed());
            return Ok(QueryResult {
                rows: Vec::new(),
                columns: Vec::new(),
                special_fields: ::protobuf::SpecialFields::default(),
            });
        }
        
        let schema = batches[0].schema();
        
        // 检查 schema 是否有字段
        if schema.fields().is_empty() {
            log::warn!("[SQL] 查询结果 schema 为空，总耗时: {:?}", start_total.elapsed());
            return Ok(QueryResult {
                rows: Vec::new(),
                columns: Vec::new(),
                special_fields: ::protobuf::SpecialFields::default(),
            });
        }
        
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        log::info!("[SQL] 步骤5 - 结果转换: {} 个批次, 共 {} 行, schema: {:?}", batches.len(), total_rows, schema);
        
        let mut all_rows = Vec::new();
        for batch in batches {
            let batch_result = self.arrow_to_query_result(&schema, &batch, &[]);
            all_rows.extend(batch_result.rows);
        }
        
        let columns: Vec<String> = schema.fields().iter()
            .map(|f| f.name().to_string())
            .collect();
        
        let result = QueryResult {
            rows: all_rows,
            columns,
            special_fields: ::protobuf::SpecialFields::default(),
        };
        
        log::info!("[SQL] 步骤5 - 结果转换完成，耗时: {:?}", start.elapsed());
        log::info!("[SQL] 查询执行完成，总耗时: {:?}", start_total.elapsed());
        
        Ok(result)
    }
    
    async fn register_table(&mut self, table_name: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.storage_engine.read().await;
        let table_provider = engine.create_table_provider(table_name);
        drop(engine);
        
        self.ctx.register_table(TableReference::bare(table_name), table_provider)?;
        Ok(())
    }
    
    async fn register_table_with_schema(&mut self, schema: &str, table_name: &str, table_provider: Arc<dyn datafusion::datasource::TableProvider>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let table_ref = CommonTableReference::full("laoflchdb", schema, table_name);
        
        if self.ctx.catalog("laoflchdb").and_then(|c| c.schema(schema)).is_none() {
            log::info!("[SQL] Creating schema '{}' in catalog 'laoflchdb'", schema);
            let sql = format!("CREATE SCHEMA IF NOT EXISTS laoflchdb.{};", schema);
            if let Err(e) = self.ctx.sql(&sql).await {
                log::warn!("[SQL] Failed to create schema '{}': {}", schema, e);
                return Ok(());
            }
        }
        
        // 先取消注册旧表（如果存在），然后重新注册
        let _ = self.ctx.deregister_table(table_ref.clone());
        
        match self.ctx.register_table(table_ref, table_provider) {
            Ok(_) => log::info!("[SQL] Registered table '{}.{}'", schema, table_name),
            Err(e) => log::warn!("[SQL] Failed to register table '{}.{}': {}", schema, table_name, e),
        }
        Ok(())
    }
    
    async fn refresh_tables(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.storage_engine.read().await;
        let schema_name = StorageEngine::get_schema_name(&*engine);
        let tables = StorageEngine::list_tables(&*engine).await?;
        
        for table in tables {
            let table_provider = engine.create_table_provider(&table);
            let table_ref = CommonTableReference::full("laoflchdb", schema_name, table.as_str());
            self.ctx.register_table(table_ref, table_provider)?;
        }
        
        Ok(())
    }
    
    async fn deregister_table(&mut self, table_name: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.ctx.deregister_table(TableReference::bare(table_name))?;
        Ok(())
    }
    
    async fn deregister_table_with_schema(&mut self, schema: &str, table_name: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let table_ref = CommonTableReference::full("laoflchdb", schema, table_name);
        let _ = self.ctx.deregister_table(table_ref);
        Ok(())
    }
}