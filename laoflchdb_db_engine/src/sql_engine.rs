use std::sync::Arc;

use datafusion::arrow::array::{ArrayRef, StringArray, Int64Array, Float64Array, BinaryArray};
use datafusion::arrow::datatypes::{DataType, Field as ArrowField, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::datasource::MemTable;
use datafusion::execution::context::{SessionContext, SessionConfig};
use datafusion::sql::TableReference;
use protobuf::Message;

use crate::{StorageEngine, SQLEngine, QueryResult, QueryRow};
use crate::field::Field as PbField;
use crate::field::field::Value;
use crate::metadata::ColumnType;

#[inline]
fn parse_field_from_bytes(field_bytes: &[u8]) -> Result<PbField, protobuf::Error> {
    let mut input = protobuf::CodedInputStream::from_bytes(field_bytes);
    PbField::parse_from(&mut input)
}

pub struct DataFusionSQLEngine<E: StorageEngine> {
    storage_engine: Arc<tokio::sync::RwLock<E>>,
    ctx: SessionContext,
}

impl<E: StorageEngine> DataFusionSQLEngine<E> {
    pub fn new(storage_engine: Arc<tokio::sync::RwLock<E>>) -> Self {
        let config = SessionConfig::new();
        let ctx = SessionContext::new_with_config(config);
        
        Self {
            storage_engine,
            ctx,
        }
    }
    
    fn column_type_to_arrow_type(col_type: &ColumnType) -> DataType {
        match col_type {
            ColumnType::COLUMN_TYPE_STRING => DataType::Utf8,
            ColumnType::COLUMN_TYPE_INT64 => DataType::Int64,
            ColumnType::COLUMN_TYPE_FLOAT => DataType::Float64,
            ColumnType::COLUMN_TYPE_BYTES => DataType::Binary,
            _ => DataType::Utf8,
        }
    }
    
    fn get_enum_value(col_type: &ColumnType) -> i32 {
        *col_type as i32
    }
    
    async fn table_to_arrow(&self, table_name: &str) -> Result<(Schema, Vec<ArrayRef>, Vec<(i32, String)>), Box<dyn std::error::Error + Send + Sync>> {
        let engine: tokio::sync::RwLockReadGuard<'_, E> = self.storage_engine.read().await;
        let columns = engine.list_table_cols(table_name).await?;
        let rows = engine.scan_table(table_name, None).await?;
        
        let mut column_infos: Vec<(i32, String)> = Vec::new();
        let mut arrow_fields = Vec::new();
        let mut arrow_arrays: Vec<Vec<ArrayRef>> = Vec::new();
        
        for col in &columns {
            let col_type = col.column_type.enum_value_or_default();
            let data_type = Self::column_type_to_arrow_type(&col_type);
            column_infos.push((Self::get_enum_value(&col_type), col.column_name.clone()));
            arrow_fields.push(ArrowField::new(&col.column_name, data_type, true));
            arrow_arrays.push(Vec::new());
        }
        
        for (_, row) in rows {
            for (idx, field_bytes) in row.data.iter().enumerate() {
                if idx >= arrow_arrays.len() {
                    break;
                }
                
                let pb_field = match parse_field_from_bytes(field_bytes) {
                    Ok(f) => f,
                    Err(_) => continue,
                };
                
                let array = match pb_field.value {
                    Some(Value::StringValue(s)) => {
                        Arc::new(StringArray::from(vec![s.value.clone()])) as ArrayRef
                    }
                    Some(Value::IntegerValue(i)) => {
                        Arc::new(Int64Array::from(vec![i.value])) as ArrayRef
                    }
                    Some(Value::FloatValue(f)) => {
                        Arc::new(Float64Array::from(vec![f.value])) as ArrayRef
                    }
                    Some(Value::BytesValue(b)) => {
                        Arc::new(BinaryArray::from(vec![b.value.as_slice()])) as ArrayRef
                    }
                    _ => Arc::new(StringArray::from(vec![""])) as ArrayRef,
                };
                arrow_arrays[idx].push(array);
            }
        }
        
        let merged_arrays: Vec<ArrayRef> = arrow_arrays.into_iter()
            .enumerate()
            .map(|(idx, arrays)| {
                if arrays.is_empty() {
                    let (col_type, _) = &column_infos[idx];
                    match *col_type {
                        0 => Arc::new(StringArray::from(Vec::<String>::new())) as ArrayRef,
                        1 => Arc::new(Int64Array::from(Vec::<i64>::new())) as ArrayRef,
                        3 => Arc::new(Float64Array::from(Vec::<f64>::new())) as ArrayRef,
                        2 => Arc::new(BinaryArray::from(Vec::<&[u8]>::new())) as ArrayRef,
                        _ => Arc::new(StringArray::from(Vec::<String>::new())) as ArrayRef,
                    }
                } else if arrays.len() == 1 {
                    arrays[0].clone()
                } else {
                    let refs: Vec<&dyn datafusion::arrow::array::Array> = arrays.iter().map(|a| a.as_ref()).collect();
                    datafusion::arrow::compute::concat(refs.as_slice()).unwrap()
                }
            })
            .collect();
        
        let schema = Schema::new(arrow_fields);
        Ok((schema, merged_arrays, column_infos))
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
                        pb_field.value = Some(Value::StringValue(crate::field::String {
                            value: value.to_string(),
                            special_fields: ::protobuf::SpecialFields::default(),
                        }));
                    }
                    DataType::Int64 => {
                        let array = array.as_any().downcast_ref::<Int64Array>().unwrap();
                        let value = array.value(i);
                        pb_field.value = Some(Value::IntegerValue(crate::field::Integer {
                            value,
                            special_fields: ::protobuf::SpecialFields::default(),
                        }));
                    }
                    DataType::Float64 => {
                        let array = array.as_any().downcast_ref::<Float64Array>().unwrap();
                        let value = array.value(i);
                        pb_field.value = Some(Value::FloatValue(crate::field::Float {
                            value,
                            special_fields: ::protobuf::SpecialFields::default(),
                        }));
                    }
                    DataType::Binary => {
                        let array = array.as_any().downcast_ref::<BinaryArray>().unwrap();
                        let value = array.value(i);
                        pb_field.value = Some(Value::BytesValue(crate::field::Bytes {
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
            let row = crate::row::Row {
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
impl<E: StorageEngine + 'static> SQLEngine for DataFusionSQLEngine<E> {
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
        let (_, _, column_infos) = self.table_to_arrow("").await?;
        let result = self.arrow_to_query_result(&schema, &batches[0], &column_infos);
        Ok(result)
    }
    
    async fn register_table(&mut self, table_name: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.storage_engine.read().await;
        let columns = engine.list_table_cols(table_name).await?;
        let rows = engine.scan_table(table_name, None).await?;
        
        let mut column_infos: Vec<(i32, String)> = Vec::new();
        let mut arrow_fields = Vec::new();
        let mut arrow_arrays: Vec<Vec<ArrayRef>> = Vec::new();
        
        for col in &columns {
            let col_type = col.column_type.enum_value_or_default();
            let data_type = Self::column_type_to_arrow_type(&col_type);
            column_infos.push((Self::get_enum_value(&col_type), col.column_name.clone()));
            arrow_fields.push(ArrowField::new(&col.column_name, data_type, true));
            arrow_arrays.push(Vec::new());
        }
        
        for (_, row) in rows {
            for (idx, field_bytes) in row.data.iter().enumerate() {
                if idx >= arrow_arrays.len() {
                    break;
                }
                
                let pb_field = match parse_field_from_bytes(field_bytes) {
                    Ok(f) => f,
                    Err(_) => continue,
                };
                
                let array = match pb_field.value {
                    Some(Value::StringValue(s)) => {
                        Arc::new(StringArray::from(vec![s.value.clone()])) as ArrayRef
                    }
                    Some(Value::IntegerValue(i)) => {
                        Arc::new(Int64Array::from(vec![i.value])) as ArrayRef
                    }
                    Some(Value::FloatValue(f)) => {
                        Arc::new(Float64Array::from(vec![f.value])) as ArrayRef
                    }
                    Some(Value::BytesValue(b)) => {
                        Arc::new(BinaryArray::from(vec![b.value.as_slice()])) as ArrayRef
                    }
                    _ => Arc::new(StringArray::from(vec![""])) as ArrayRef,
                };
                arrow_arrays[idx].push(array);
            }
        }
        
        let merged_arrays: Vec<ArrayRef> = arrow_arrays.into_iter()
            .enumerate()
            .map(|(idx, arrays)| {
                if arrays.is_empty() {
                    let (col_type, _) = &column_infos[idx];
                    match *col_type {
                        0 => Arc::new(StringArray::from(Vec::<String>::new())) as ArrayRef,
                        1 => Arc::new(Int64Array::from(Vec::<i64>::new())) as ArrayRef,
                        3 => Arc::new(Float64Array::from(Vec::<f64>::new())) as ArrayRef,
                        2 => Arc::new(BinaryArray::from(Vec::<&[u8]>::new())) as ArrayRef,
                        _ => Arc::new(StringArray::from(Vec::<String>::new())) as ArrayRef,
                    }
                } else if arrays.len() == 1 {
                    arrays[0].clone()
                } else {
                    let refs: Vec<&dyn datafusion::arrow::array::Array> = arrays.iter().map(|a| a.as_ref()).collect();
                    datafusion::arrow::compute::concat(refs.as_slice()).unwrap()
                }
            })
            .collect();
        
        let schema = Schema::new(arrow_fields);
        let batch = RecordBatch::try_new(Arc::new(schema), merged_arrays)?;
        
        let table = MemTable::try_new(batch.schema().clone(), vec![vec![batch]])?;
        self.ctx.register_table(TableReference::bare(table_name), Arc::new(table))?;
        
        Ok(())
    }
    
    async fn refresh_tables(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.storage_engine.read().await;
        let tables = engine.list_tables().await?;
        
        for table in tables {
            let engine = self.storage_engine.read().await;
            let columns = engine.list_table_cols(&table).await?;
            let rows = engine.scan_table(&table, None).await?;
            
            let mut column_infos: Vec<(i32, String)> = Vec::new();
            let mut arrow_fields = Vec::new();
            let mut arrow_arrays: Vec<Vec<ArrayRef>> = Vec::new();
            
            for col in &columns {
                let col_type = col.column_type.enum_value_or_default();
                let data_type = Self::column_type_to_arrow_type(&col_type);
                column_infos.push((Self::get_enum_value(&col_type), col.column_name.clone()));
                arrow_fields.push(ArrowField::new(&col.column_name, data_type, true));
                arrow_arrays.push(Vec::new());
            }
            
            for (_, row) in rows {
                for (idx, field_bytes) in row.data.iter().enumerate() {
                    if idx >= arrow_arrays.len() {
                        break;
                    }
                    
                    let pb_field = match parse_field_from_bytes(field_bytes) {
                        Ok(f) => f,
                        Err(_) => continue,
                    };
                    
                    let array = match pb_field.value {
                        Some(Value::StringValue(s)) => {
                            Arc::new(StringArray::from(vec![s.value.clone()])) as ArrayRef
                        }
                        Some(Value::IntegerValue(i)) => {
                            Arc::new(Int64Array::from(vec![i.value])) as ArrayRef
                        }
                        Some(Value::FloatValue(f)) => {
                            Arc::new(Float64Array::from(vec![f.value])) as ArrayRef
                        }
                        Some(Value::BytesValue(b)) => {
                            Arc::new(BinaryArray::from(vec![b.value.as_slice()])) as ArrayRef
                        }
                        _ => Arc::new(StringArray::from(vec![""])) as ArrayRef,
                    };
                    arrow_arrays[idx].push(array);
                }
            }
            
            let merged_arrays: Vec<ArrayRef> = arrow_arrays.into_iter()
                .enumerate()
                .map(|(idx, arrays)| {
                    if arrays.is_empty() {
                        let (col_type, _) = &column_infos[idx];
                        match *col_type {
                            0 => Arc::new(StringArray::from(Vec::<String>::new())) as ArrayRef,
                            1 => Arc::new(Int64Array::from(Vec::<i64>::new())) as ArrayRef,
                            3 => Arc::new(Float64Array::from(Vec::<f64>::new())) as ArrayRef,
                            2 => Arc::new(BinaryArray::from(Vec::<&[u8]>::new())) as ArrayRef,
                            _ => Arc::new(StringArray::from(Vec::<String>::new())) as ArrayRef,
                        }
                    } else if arrays.len() == 1 {
                        arrays[0].clone()
                    } else {
                        let refs: Vec<&dyn datafusion::arrow::array::Array> = arrays.iter().map(|a| a.as_ref()).collect();
                        datafusion::arrow::compute::concat(refs.as_slice()).unwrap()
                    }
                })
                .collect();
            
            let schema = Schema::new(arrow_fields);
            let batch = RecordBatch::try_new(Arc::new(schema), merged_arrays)?;
            
            let table_impl = MemTable::try_new(batch.schema().clone(), vec![vec![batch]])?;
            self.ctx.register_table(TableReference::bare(&*table), Arc::new(table_impl))?;
        }
        
        Ok(())
    }
}
