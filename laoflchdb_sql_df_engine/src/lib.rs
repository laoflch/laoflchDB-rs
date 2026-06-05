use std::sync::Arc;

use datafusion::arrow::array::{StringArray, Int64Array, Float64Array, BinaryArray};
use datafusion::arrow::datatypes::{DataType, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::datasource::TableProvider;
use datafusion::execution::context::{SessionContext, SessionConfig};
use datafusion::logical_expr::Expr;
use datafusion::sql::TableReference;
use protobuf::Message;

use laoflchdb_engines::{StorageEngine, SQLEngine, QueryResult, QueryRow};
use laoflchdb_engines::field::Field as PbField;
use laoflchdb_engines::field::field::Value;

#[async_trait::async_trait]
pub trait DataFusionStorageEngine: Send + Sync + 'static {
    async fn table_to_arrow(&self, table_name: &str) -> Result<(Schema, Vec<datafusion::arrow::array::ArrayRef>, Vec<(i32, String)>), Box<dyn std::error::Error + Send + Sync>>;
    
    fn create_table_provider(&self, table_name: &str) -> Arc<dyn TableProvider>;
}

pub struct DataFusionSQLEngine<E: StorageEngine + DataFusionStorageEngine> {
    storage_engine: Arc<tokio::sync::RwLock<E>>,
    ctx: SessionContext,
}



impl<E: StorageEngine + DataFusionStorageEngine> DataFusionSQLEngine<E> {
    pub fn new(storage_engine: Arc<tokio::sync::RwLock<E>>) -> Self {
        let config = SessionConfig::new();
        let ctx = SessionContext::new_with_config(config);
        
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
        let ctx = self.ctx.clone();
        let sql = sql.to_string();
        
        let df = ctx.sql(sql.as_str()).await.map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        
        println!("\n===== Logical Plan =====");
        println!("{}", df.logical_plan());
        
        let physical_plan = df.clone().create_physical_plan().await.map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        println!("\n===== Physical Plan =====");
        println!("{:?}", physical_plan);
        
        println!("\n===== Optimization Tips =====");
        println!("1. Filter pushdown: Check if WHERE conditions are pushed to storage");
        println!("2. Projection pushdown: Check if only needed columns are scanned");
        println!("3. Limit pushdown: Check if LIMIT is applied early");
        println!("4. Join optimization: Check join order and type");
        
        let batches = df.collect().await.map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        
        if batches.is_empty() {
            return Ok(QueryResult {
                rows: Vec::new(),
                columns: Vec::new(),
                special_fields: ::protobuf::SpecialFields::default(),
            });
        }
        
        let schema = batches[0].schema();
        let result = self.arrow_to_query_result(&schema, &batches[0], &[]);
        Ok(result)
    }
    
    async fn register_table(&mut self, table_name: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.storage_engine.read().await;
        let table_provider = engine.create_table_provider(table_name);
        drop(engine);
        
        self.ctx.register_table(TableReference::bare(table_name), table_provider)?;
        Ok(())
    }
    
    async fn refresh_tables(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.storage_engine.read().await;
        let tables = StorageEngine::list_tables(&*engine).await?;
        
        for table in tables {
            let table_provider = engine.create_table_provider(&table);
            self.ctx.register_table(TableReference::bare(&*table), table_provider)?;
        }
        
        Ok(())
    }
}