use std::sync::Arc;

use datafusion::arrow::array::{StringArray, Int64Array, Float64Array, BinaryArray};
use datafusion::arrow::datatypes::{DataType, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::datasource::MemTable;
use datafusion::execution::context::{SessionContext, SessionConfig};
use datafusion::sql::TableReference;
use protobuf::Message;

use laoflchdb_engines::{DataFusionStorageEngine, StorageEngine, SQLEngine, QueryResult, QueryRow};
use laoflchdb_engines::field::Field as PbField;
use laoflchdb_engines::field::field::Value;

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
                
                let mut pb_field = PbField::new();
                
                match field.data_type() {
                    DataType::Utf8 => {
                        let array = array.as_any().downcast_ref::<StringArray>().unwrap();
                        let value = array.value(i);
                        pb_field.value = Some(Value::StringValue(laoflchdb_engines::field::String {
                            value: value.to_string(),
                            special_fields: ::protobuf::SpecialFields::default(),
                        }));
                    }
                    DataType::Int64 => {
                        let array = array.as_any().downcast_ref::<Int64Array>().unwrap();
                        let value = array.value(i);
                        pb_field.value = Some(Value::IntegerValue(laoflchdb_engines::field::Integer {
                            value,
                            special_fields: ::protobuf::SpecialFields::default(),
                        }));
                    }
                    DataType::Float64 => {
                        let array = array.as_any().downcast_ref::<Float64Array>().unwrap();
                        let value = array.value(i);
                        pb_field.value = Some(Value::FloatValue(laoflchdb_engines::field::Float {
                            value,
                            special_fields: ::protobuf::SpecialFields::default(),
                        }));
                    }
                    DataType::Binary => {
                        let array = array.as_any().downcast_ref::<BinaryArray>().unwrap();
                        let value = array.value(i);
                        pb_field.value = Some(Value::BytesValue(laoflchdb_engines::field::Bytes {
                            value: value.to_vec(),
                            special_fields: ::protobuf::SpecialFields::default(),
                        }));
                    }
                    _ => {}
                };
                
                let mut buf = Vec::new();
                pb_field.write_to_vec(&mut buf).unwrap();
                row_data.push(buf);
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
        
        QueryResult {
            rows: query_rows,
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
        let batches = df.collect().await.map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        
        if batches.is_empty() {
            return Ok(QueryResult {
                rows: Vec::new(),
                special_fields: ::protobuf::SpecialFields::default(),
            });
        }
        
        let schema = batches[0].schema();
        let (_, _, column_infos) = self.storage_engine.read().await.table_to_arrow("").await?;
        let result = self.arrow_to_query_result(&schema, &batches[0], &column_infos);
        Ok(result)
    }
    
    async fn register_table(&mut self, table_name: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.storage_engine.read().await;
        let (schema, merged_arrays, _) = engine.table_to_arrow(table_name).await?;
        drop(engine);
        
        let batch = RecordBatch::try_new(Arc::new(schema), merged_arrays)?;
        let table = MemTable::try_new(batch.schema().clone(), vec![vec![batch]])?;
        self.ctx.register_table(TableReference::bare(table_name), Arc::new(table))?;
        
        Ok(())
    }
    
    async fn refresh_tables(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.storage_engine.read().await;
        let tables = StorageEngine::list_tables(&*engine).await?;
        drop(engine);
        
        for table in tables {
            let engine = self.storage_engine.read().await;
            let (schema, merged_arrays, _) = engine.table_to_arrow(&table).await?;
            drop(engine);
            
            let batch = RecordBatch::try_new(Arc::new(schema), merged_arrays)?;
            let table_impl = MemTable::try_new(batch.schema().clone(), vec![vec![batch]])?;
            self.ctx.register_table(TableReference::bare(&*table), Arc::new(table_impl))?;
        }
        
        Ok(())
    }
}