use std::collections::HashMap;
use std::sync::{Mutex, RwLock};
use std::path::Path;

use async_trait::async_trait;
use log::{info, warn, debug};
use snowflake_me::Snowflake;
use tantivy::{
    collector::TopDocs,
    directory::MmapDirectory,
    doc,
    query::{QueryParser, Occur},
    schema::{self, Field, Schema, TextOptions, NumericOptions, IndexRecordOption, Value},
    Index, IndexReader, IndexWriter, IndexSettings, DocAddress,
};
use uuid::Uuid;
use thiserror::Error;

use laoflchdb_engines::{
    ColumnMeta, ColumnType, Query, QueryResult, Row, 
    StorageEngine, TableMeta, EnumOrUnknown, SpecialFields, RowType,
};

#[derive(Error, Debug)]
pub enum TantivyEngineError {
    #[error("Table not found: {0}")]
    TableNotFound(String),
    #[error("Row not found: {0}")]
    RowNotFound(u64),
    #[error("Column not found: {0}")]
    ColumnNotFound(String),
    #[error("Tantivy error: {0}")]
    TantivyError(#[from] tantivy::TantivyError),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("Lock error: {0}")]
    LockError(String),
    #[error("Query parse error: {0}")]
    QueryParseError(String),
}

struct TableIndex {
    index: Index,
    writer: IndexWriter,
    schema: Schema,
    field_map: HashMap<String, Field>,
}

pub struct TantivyStorageEngine {
    base_path: String,
    schema_name: String,
    tables: RwLock<HashMap<String, TableMeta>>,
    table_indices: RwLock<HashMap<String, Mutex<TableIndex>>>,
    next_row_id: RwLock<HashMap<String, u64>>,
    snowflake: Mutex<Snowflake>,
}

impl TantivyStorageEngine {
    pub fn new(base_path: &str, schema_name: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let path = Path::new(base_path);
        if !path.exists() {
            std::fs::create_dir_all(path)?;
        }

        let snowflake = Snowflake::new()?;

        Ok(Self {
            base_path: base_path.to_string(),
            schema_name: schema_name.to_string(),
            tables: RwLock::new(HashMap::new()),
            table_indices: RwLock::new(HashMap::new()),
            next_row_id: RwLock::new(HashMap::new()),
            snowflake: Mutex::new(snowflake),
        })
    }

    pub fn get_schema_name(&self) -> &str {
        &self.schema_name
    }

    fn create_tantivy_schema(&self, columns: &[(u32, &str, ColumnType, Option<&str>)]) -> Schema {
        let mut schema_builder = Schema::builder();

        schema_builder.add_u64_field("_row_id", NumericOptions::default().set_stored().set_indexed());

        for (_idx, name, col_type, _comment) in columns {
            match col_type {
                ColumnType::COLUMN_TYPE_STRING | ColumnType::COLUMN_TYPE_BYTES => {
                    let text_options = TextOptions::default()
                        .set_indexing_options(
                            schema::TextFieldIndexing::default()
                                .set_tokenizer("en_stem")
                                .set_index_option(schema::IndexRecordOption::WithFreqsAndPositions)
                        )
                        .set_stored();
                    schema_builder.add_text_field(name, text_options);
                }
                ColumnType::COLUMN_TYPE_INT64 => {
                    let int_options = NumericOptions::default().set_stored();
                    schema_builder.add_i64_field(name, int_options);
                }
                ColumnType::COLUMN_TYPE_FLOAT => {
                    let float_options = NumericOptions::default().set_stored();
                    schema_builder.add_f64_field(name, float_options);
                }
                _ => {
                    let text_options = TextOptions::default().set_stored();
                    schema_builder.add_text_field(name, text_options);
                }
            }
        }

        schema_builder.build()
    }

    fn get_next_row_id(&self, table: &str) -> u64 {
        match self.snowflake.lock() {
            Ok(mut snowflake_guard) => {
                match snowflake_guard.next_id() {
                    Ok(id) => {
                        info!("Generated Snowflake ID: {} for table: {}", id, table);
                        id
                    },
                    Err(e) => {
                        let error_msg = format!("{:?}", e);
                        info!("Snowflake ID generation failed: {}, falling back to auto-increment ID", error_msg);
                        let mut next_row_id_guard = self.next_row_id.write().unwrap();
                        let next_id = next_row_id_guard.entry(table.to_string()).or_insert(1);
                        let current_id = *next_id;
                        *next_id += 1;
                        info!("Generated auto-increment ID: {} for table: {}", current_id, table);
                        current_id
                    }
                }
            },
            Err(e) => {
                info!("Failed to lock Snowflake mutex: {:?}, falling back to auto-increment ID", e);
                let mut next_row_id_guard = self.next_row_id.write().unwrap();
                let next_id = next_row_id_guard.entry(table.to_string()).or_insert(1);
                let current_id = *next_id;
                *next_id += 1;
                info!("Generated auto-increment ID: {} for table: {}", current_id, table);
                current_id
            }
        }
    }
}

#[async_trait]
impl StorageEngine for TantivyStorageEngine {
    async fn create_table(&mut self, table: &str, table_comment: Option<&str>, columns: &[(u32, &str, ColumnType, Option<&str>)]) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        let table_id = Uuid::new_v4().as_u128() as u64;
        let column_count = columns.len() as u32;

        let tables_guard = self.tables.read().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock tables: {}", e)))?;
        if tables_guard.contains_key(table) {
            drop(tables_guard);
            return Ok(table_id);
        }
        drop(tables_guard);

        let tantivy_schema = self.create_tantivy_schema(columns);
        
        let mut field_map: HashMap<String, Field> = HashMap::new();
        for (_idx, name, _col_type, _comment) in columns {
            if let Ok(field) = tantivy_schema.get_field(name) {
                field_map.insert(name.to_string(), field);
            }
        }

        let row_id_field = tantivy_schema.get_field("_row_id").unwrap();
        field_map.insert("_row_id".to_string(), row_id_field);

        let table_path = Path::new(&self.base_path).join(table);
        if table_path.exists() {
            return Ok(table_id);
        }
        std::fs::create_dir_all(&table_path)?;

        let directory = MmapDirectory::open(&table_path)?;
        let index = Index::create(directory, tantivy_schema.clone(), IndexSettings::default())?;
        let writer = index.writer(50_000_000)?;

        let table_index = TableIndex {
            index,
            writer,
            schema: tantivy_schema,
            field_map,
        };

        let table_meta = TableMeta {
            table_id,
            table_name: table.to_string(),
            column_count,
            comment: table_comment.unwrap_or("").to_string(),
            next_auto_inc_column_id: columns.len() as u64,
            special_fields: SpecialFields::new(),
        };

        let mut tables_guard = self.tables.write().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock tables: {}", e)))?;
        tables_guard.insert(table.to_string(), table_meta);

        let mut table_indices_guard = self.table_indices.write().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock table indices: {}", e)))?;
        table_indices_guard.insert(table.to_string(), Mutex::new(table_index));

        let mut next_row_id_guard = self.next_row_id.write().unwrap();
        next_row_id_guard.insert(table.to_string(), 1);

        info!("Created table '{}' with {} columns in schema '{}'", table, column_count, self.schema_name);
        Ok(table_id)
    }

    async fn drop_table(&mut self, table: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let tables_guard = self.tables.read().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock tables: {}", e)))?;
        if !tables_guard.contains_key(table) {
            return Ok(());
        }
        drop(tables_guard);

        let mut tables_guard = self.tables.write().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock tables: {}", e)))?;
        tables_guard.remove(table);

        let mut table_indices_guard = self.table_indices.write().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock table indices: {}", e)))?;
        table_indices_guard.remove(table);

        let mut next_row_id_guard = self.next_row_id.write().unwrap();
        next_row_id_guard.remove(table);

        let table_path = Path::new(&self.base_path).join(table);
        if table_path.exists() {
            std::fs::remove_dir_all(&table_path)?;
        }

        info!("Dropped table '{}'", table);
        Ok(())
    }

    async fn list_tables(&self) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        let tables_guard = self.tables.read().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock tables: {}", e)))?;
        let tables: Vec<String> = tables_guard.keys().cloned().collect();
        Ok(tables)
    }

    async fn list_table_cols(&self, table: &str) -> Result<Vec<ColumnMeta>, Box<dyn std::error::Error + Send + Sync>> {
        let table_indices_guard = self.table_indices.read().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock table indices: {}", e)))?;
        let table_index_mutex = table_indices_guard.get(table).ok_or(TantivyEngineError::TableNotFound(table.to_string()))?;
        let table_index = table_index_mutex.lock().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock table index: {}", e)))?;

        let mut columns: Vec<ColumnMeta> = Vec::new();
        let mut column_id_counter: u64 = 0;
        
        for (field, field_entry) in table_index.schema.fields() {
            let name = field_entry.name().to_string();
            if name == "_row_id" {
                continue;
            }
            
            let col_type = match field_entry.field_type() {
                schema::FieldType::Str(_) => ColumnType::COLUMN_TYPE_STRING,
                schema::FieldType::I64(_) => ColumnType::COLUMN_TYPE_INT64,
                schema::FieldType::F64(_) => ColumnType::COLUMN_TYPE_FLOAT,
                _ => ColumnType::COLUMN_TYPE_STRING,
            };
            
            columns.push(ColumnMeta {
                table_id: 0,
                column_id: column_id_counter,
                column_name: name,
                column_type: EnumOrUnknown::from(col_type),
                comment: String::new(),
                special_fields: SpecialFields::new(),
            });
            column_id_counter += 1;
        }

        Ok(columns)
    }

    async fn add_row(&mut self, table: &str, row: &Row) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        {
            let tables_guard = self.tables.read().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock tables: {}", e)))?;
            if !tables_guard.contains_key(table) {
                return Err(Box::new(TantivyEngineError::TableNotFound(table.to_string())));
            }
        }

        let row_id = self.get_next_row_id(table);
        let cols = self.list_table_cols(table).await?;

        {
            let table_indices_guard = self.table_indices.write().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock table indices: {}", e)))?;
            let table_index_mutex = table_indices_guard.get(table).ok_or(TantivyEngineError::TableNotFound(table.to_string()))?;
            let mut table_index = table_index_mutex.lock().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock table index: {}", e)))?;

            let mut doc_builder = doc!();
            
            if let Some(row_id_field) = table_index.field_map.get("_row_id") {
                doc_builder.add_u64(*row_id_field, row_id);
            }
            
            for (idx, col) in cols.iter().enumerate() {
                if let Some(data) = row.data.get(idx) {
                    if let Some(field) = table_index.field_map.get(&col.column_name) {
                        let value_str = String::from_utf8_lossy(data);
                        doc_builder.add_text(*field, value_str.to_string());
                    }
                }
            }

            table_index.writer.add_document(doc_builder)?;
            table_index.writer.commit()?;
        }
        
        debug!("Added row {} to table '{}'", row_id, table);
        Ok(row_id)
    }

    async fn get_row(&self, table: &str, row_id: u64) -> Result<Option<Row>, Box<dyn std::error::Error + Send + Sync>> {
        let doc_address: Option<DocAddress>;
        
        {
            let table_indices_guard = self.table_indices.read().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock table indices: {}", e)))?;
            let table_index_mutex = table_indices_guard.get(table).ok_or(TantivyEngineError::TableNotFound(table.to_string()))?;
            let table_index = table_index_mutex.lock().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock table index: {}", e)))?;

            let row_id_field = match table_index.schema.get_field("_row_id") {
                Ok(f) => f,
                Err(_) => return Ok(None),
            };

            let reader = table_index.index.reader()?;
            let searcher = reader.searcher();
            let term = tantivy::Term::from_field_u64(row_id_field, row_id);
            let term_query = tantivy::query::TermQuery::new(term, IndexRecordOption::Basic);
            
            let top_docs = searcher.search(&term_query, &TopDocs::with_limit(1).order_by_score())?;
            
            if top_docs.is_empty() {
                return Ok(None);
            }

            let (_score, addr) = top_docs[0];
            doc_address = Some(addr);
        }

        let cols = self.list_table_cols(table).await?;

        let row_data: Option<Vec<Vec<u8>>>;
        {
            let table_indices_guard = self.table_indices.read().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock table indices: {}", e)))?;
            let table_index_mutex = table_indices_guard.get(table).ok_or(TantivyEngineError::TableNotFound(table.to_string()))?;
            let table_index = table_index_mutex.lock().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock table index: {}", e)))?;
            let reader = table_index.index.reader()?;
            let searcher = reader.searcher();
            
            match searcher.doc::<tantivy::TantivyDocument>(doc_address.unwrap()) {
                Ok(retrieved_doc) => {
                    let mut data: Vec<Vec<u8>> = Vec::new();
                    for col in &cols {
                        if let Some(field) = table_index.field_map.get(&col.column_name) {
                            let field_entry = table_index.schema.get_field_entry(*field);
                            let field_type = field_entry.field_type();
                            
                            if let Some(value) = retrieved_doc.get_first(*field) {
                                let s = match field_type {
                                    schema::FieldType::Str(_) => {
                                        match value.as_str() {
                                            Some(s) => s.to_string(),
                                            None => {
                                                let bytes = value.as_bytes().unwrap_or_default();
                                                String::from_utf8_lossy(bytes).to_string()
                                            }
                                        }
                                    }
                                    schema::FieldType::I64(_) => {
                                        format!("{}", value.as_i64().unwrap_or(0))
                                    }
                                    schema::FieldType::U64(_) => {
                                        format!("{}", value.as_u64().unwrap_or(0))
                                    }
                                    schema::FieldType::F64(_) => {
                                        format!("{}", value.as_f64().unwrap_or(0.0))
                                    }
                                    _ => {
                                        format!("{:?}", value).trim_matches('"').to_string()
                                    }
                                };
                                data.push(s.as_bytes().to_vec());
                            } else {
                                data.push(Vec::new());
                            }
                        } else {
                            data.push(Vec::new());
                        }
                    }
                    row_data = Some(data);
                }
                Err(_) => row_data = None,
            }
        }

        match row_data {
            Some(data) => {
                let row = Row {
                    row_type: EnumOrUnknown::from(RowType::ROW_TYPE_NORMAL),
                    version: 1,
                    data,
                    special_fields: SpecialFields::new(),
                };
                Ok(Some(row))
            }
            None => Ok(None),
        }
    }

    async fn delete_row(&mut self, table: &str, row_id: u64) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        {
            let tables_guard = self.tables.read().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock tables: {}", e)))?;
            if !tables_guard.contains_key(table) {
                return Err(Box::new(TantivyEngineError::TableNotFound(table.to_string())));
            }
        }

        {
            let table_indices_guard = self.table_indices.write().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock table indices: {}", e)))?;
            let table_index_mutex = table_indices_guard.get(table).ok_or(TantivyEngineError::TableNotFound(table.to_string()))?;
            let mut table_index = table_index_mutex.lock().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock table index: {}", e)))?;

            let row_id_field = match table_index.schema.get_field("_row_id") {
                Ok(f) => f,
                Err(_) => {
                    debug!("Row {} not found in table '{}'", row_id, table);
                    return Ok(());
                }
            };

            let term = tantivy::Term::from_field_u64(row_id_field, row_id);
            table_index.writer.delete_term(term);
            table_index.writer.commit()?;
        }

        debug!("Deleted row {} from table '{}'", row_id, table);
        Ok(())
    }

    async fn update_row(&mut self, table: &str, row_id: u64, row: &Row) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.delete_row(table, row_id).await?;
        self.add_row(table, row).await?;
        Ok(())
    }

    async fn get_all_meta(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let tables_guard = self.tables.read().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock tables: {}", e)))?;
        
        let meta: Vec<serde_json::Value> = tables_guard.values()
            .map(|tm| serde_json::json!({
                "table_id": tm.table_id,
                "table_name": tm.table_name,
                "column_count": tm.column_count,
                "comment": tm.comment,
            }))
            .collect();

        Ok(serde_json::json!({
            "schema_name": self.schema_name,
            "tables": meta,
            "tables_count": tables_guard.len(),
        }).to_string())
    }

    async fn get_schema_info(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok(format!("Schema: {}, Path: {}", self.schema_name, self.base_path))
    }

    async fn get_table_meta(&self, table: &str) -> Result<Option<TableMeta>, Box<dyn std::error::Error + Send + Sync>> {
        let tables_guard = self.tables.read().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock tables: {}", e)))?;
        Ok(tables_guard.get(table).cloned())
    }

    async fn update_table_comment(&mut self, table: &str, comment: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut tables_guard = self.tables.write().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock tables: {}", e)))?;
        if let Some(table_meta) = tables_guard.get_mut(table) {
            table_meta.comment = comment.to_string();
            Ok(())
        } else {
            Err(Box::new(TantivyEngineError::TableNotFound(table.to_string())))
        }
    }

    async fn update_column_comment(&mut self, _table: &str, _column_name: &str, _comment: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    async fn query(&self, query: &Query) -> Result<QueryResult, Box<dyn std::error::Error + Send + Sync>> {
        let mut results: Vec<laoflchdb_engines::QueryRow> = Vec::new();

        for table_filter in &query.table_filters {
            let table_name = table_filter.table_name.clone();
            
            let top_docs: Vec<(f32, DocAddress)>;
            
            {
                let table_indices_guard = self.table_indices.read().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock table indices: {}", e)))?;
                let table_index_mutex = match table_indices_guard.get(&table_name) {
                    Some(m) => m,
                    None => continue,
                };
                let table_index = table_index_mutex.lock().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock table index: {}", e)))?;

                let fields: Vec<Field> = table_index.schema.fields().map(|(field, _)| field).collect();
                let query_parser = QueryParser::for_index(&table_index.index, fields);

                let mut tantivy_queries: Vec<Box<dyn tantivy::query::Query>> = Vec::new();

                for filter in &table_filter.column_filters {
                    for condition in &filter.conditions {
                        let query_str = match condition.value.as_ref() {
                            Some(f) => format!("{:?}", f),
                            _ => String::new(),
                        };

                        if !query_str.is_empty() {
                            match query_parser.parse_query(&query_str) {
                                Ok(q) => tantivy_queries.push(q),
                                Err(e) => warn!("Failed to parse query '{}': {}", query_str, e),
                            }
                        }
                    }
                }

                if tantivy_queries.is_empty() {
                    continue;
                }

                let combined_query = if tantivy_queries.len() == 1 {
                    tantivy_queries.remove(0)
                } else {
                    let subqueries: Vec<(Occur, Box<dyn tantivy::query::Query>)> = 
                        tantivy_queries.into_iter().map(|q| (Occur::Must, q)).collect();
                    Box::new(tantivy::query::BooleanQuery::new(subqueries))
                };

                let reader = table_index.index.reader()?;
            let searcher = reader.searcher();
                top_docs = searcher.search(&*combined_query, &TopDocs::with_limit(100).order_by_score())?;
            }

            let cols = self.list_table_cols(&table_name).await?;

            for (_score, doc_address) in top_docs {
                let table_indices_guard = self.table_indices.read().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock table indices: {}", e)))?;
                let table_index_mutex = match table_indices_guard.get(&table_name) {
                    Some(m) => m,
                    None => continue,
                };
                let table_index = table_index_mutex.lock().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock table index: {}", e)))?;
                let reader = table_index.index.reader()?;
            let searcher = reader.searcher();
                
                match searcher.doc::<tantivy::TantivyDocument>(doc_address) {
                    Ok(retrieved_doc) => {
                        let mut row_data: Vec<Vec<u8>> = Vec::new();

                        for col in &cols {
                            if let Some(field) = table_index.field_map.get(&col.column_name) {
                                if let Some(value) = retrieved_doc.get_first(*field) {
                                    row_data.push(format!("{:?}", value).as_bytes().to_vec());
                                } else {
                                    row_data.push(Vec::new());
                                }
                            } else {
                                row_data.push(Vec::new());
                            }
                        }

                        let row = Row {
                            row_type: EnumOrUnknown::from(RowType::ROW_TYPE_NORMAL),
                            version: 1,
                            data: row_data,
                            special_fields: SpecialFields::new(),
                        };

                        results.push(laoflchdb_engines::QueryRow {
                            table_name: table_name.clone(),
                            row_id: doc_address.doc_id as u64,
                            row: Some(row).into(),
                            special_fields: SpecialFields::new(),
                        });
                    }
                    Err(e) => warn!("Failed to retrieve document {}: {}", doc_address.doc_id, e),
                }
            }
        }

        Ok(QueryResult {
            rows: results,
            columns: Vec::new(),
            special_fields: SpecialFields::new(),
        })
    }

    async fn scan_table(&self, table: &str, limit: Option<usize>) -> Result<Vec<(u64, Row)>, Box<dyn std::error::Error + Send + Sync>> {
        {
            let tables_guard = self.tables.read().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock tables: {}", e)))?;
            if !tables_guard.contains_key(table) {
                return Err(Box::new(TantivyEngineError::TableNotFound(table.to_string())));
            }
        }

        let cols = self.list_table_cols(table).await?;

        let table_indices_guard = self.table_indices.read().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock table indices: {}", e)))?;
        let table_index_mutex = table_indices_guard.get(table).ok_or(TantivyEngineError::TableNotFound(table.to_string()))?;
        let table_index = table_index_mutex.lock().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock table index: {}", e)))?;

        let reader = table_index.index.reader()?;
        let searcher = reader.searcher();

        let all_query = tantivy::query::AllQuery;
        let top_docs = searcher.search(&all_query, &TopDocs::with_limit(limit.unwrap_or(10000)).order_by_score())?;
        
        let mut results: Vec<(u64, Row)> = Vec::new();

        for (_score, doc_address) in top_docs {
            match searcher.doc::<tantivy::TantivyDocument>(doc_address) {
                Ok(retrieved_doc) => {
                    let mut row_data: Vec<Vec<u8>> = Vec::new();
                    let mut row_id: u64 = doc_address.doc_id as u64;

                    for col in &cols {
                        if let Some(field) = table_index.field_map.get(&col.column_name) {
                            if col.column_name == "_row_id" {
                                if let Some(value) = retrieved_doc.get_first(*field) {
                                    row_id = value.as_u64().unwrap_or(doc_address.doc_id as u64);
                                }
                            } else {
                                let field_entry = table_index.schema.get_field_entry(*field);
                                let field_type = field_entry.field_type();
                                
                                if let Some(value) = retrieved_doc.get_first(*field) {
                                    let s = match field_type {
                                        schema::FieldType::Str(_) => {
                                            match value.as_str() {
                                                Some(s) => s.to_string(),
                                                None => {
                                                    let bytes = value.as_bytes().unwrap_or_default();
                                                    String::from_utf8_lossy(bytes).to_string()
                                                }
                                            }
                                        }
                                        schema::FieldType::I64(_) => {
                                            format!("{}", value.as_i64().unwrap_or(0))
                                        }
                                        schema::FieldType::U64(_) => {
                                            format!("{}", value.as_u64().unwrap_or(0))
                                        }
                                        schema::FieldType::F64(_) => {
                                            format!("{}", value.as_f64().unwrap_or(0.0))
                                        }
                                        _ => {
                                            format!("{:?}", value).trim_matches('"').to_string()
                                        }
                                    };
                                    row_data.push(s.as_bytes().to_vec());
                                } else {
                                    row_data.push(Vec::new());
                                }
                            }
                        } else {
                            row_data.push(Vec::new());
                        }
                    }

                    let row = Row {
                        row_type: EnumOrUnknown::from(RowType::ROW_TYPE_NORMAL),
                        version: 1,
                        data: row_data,
                        special_fields: SpecialFields::new(),
                    };

                    results.push((row_id, row));
                }
                Err(_) => continue,
            }
        }

        Ok(results)
    }

    async fn get_column_types(&self, table: &str) -> Result<HashMap<String, ColumnType>, Box<dyn std::error::Error + Send + Sync>> {
        let table_indices_guard = self.table_indices.read().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock table indices: {}", e)))?;
        let table_index_mutex = table_indices_guard.get(table).ok_or(TantivyEngineError::TableNotFound(table.to_string()))?;
        let table_index = table_index_mutex.lock().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock table index: {}", e)))?;

        let mut col_types: HashMap<String, ColumnType> = HashMap::new();
        for (name, field) in &table_index.field_map {
            if name == "_row_id" {
                continue;
            }
            let field_entry = table_index.schema.get_field_entry(*field);
            let col_type = match field_entry.field_type() {
                schema::FieldType::Str(_) => ColumnType::COLUMN_TYPE_STRING,
                schema::FieldType::I64(_) => ColumnType::COLUMN_TYPE_INT64,
                schema::FieldType::F64(_) => ColumnType::COLUMN_TYPE_FLOAT,
                _ => ColumnType::COLUMN_TYPE_STRING,
            };
            col_types.insert(name.clone(), col_type);
        }

        Ok(col_types)
    }

    async fn put(&mut self, _table: &str, _key: &[u8], _value: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    async fn get(&self, _table: &str, _key: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(None)
    }

    async fn delete(&mut self, _table: &str, _key: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("Tantivy index engine shutdown complete");
        Ok(())
    }

    fn get_schema_name(&self) -> &str {
        &self.schema_name
    }
}

impl TantivyStorageEngine {
    pub async fn search(
        &self, 
        table: &str, 
        query: &str, 
        fields: Option<&[&str]>,
        limit: usize
    ) -> Result<Vec<(u64, f32, HashMap<String, String>)>, Box<dyn std::error::Error + Send + Sync>> {
        let cols = self.list_table_cols(table).await?;
        
        let table_indices_guard = self.table_indices.read().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock table indices: {}", e)))?;
        let table_index_mutex = table_indices_guard.get(table).ok_or(TantivyEngineError::TableNotFound(table.to_string()))?;
        let table_index = table_index_mutex.lock().map_err(|e| TantivyEngineError::LockError(format!("Failed to lock table index: {}", e)))?;

        let reader = table_index.index.reader()?;
        let searcher = reader.searcher();

        let search_fields: Vec<Field> = match fields {
            Some(fs) => fs
                .iter()
                .filter(|f| **f != "_row_id")
                .filter_map(|f| table_index.field_map.get(*f).copied())
                .collect(),
            None => table_index.schema.fields()
                .filter(|(_, field_entry)| field_entry.name() != "_row_id")
                .map(|(field, _)| field)
                .collect(),
        };

        if search_fields.is_empty() {
            return Ok(vec![]);
        }

        let query_parser = QueryParser::for_index(&table_index.index, search_fields);
        
        let tantivy_query = query_parser.parse_query(query)?;
        
        let top_docs = searcher.search(&tantivy_query, &TopDocs::with_limit(limit).order_by_score())?;
        
        let mut results: Vec<(u64, f32, HashMap<String, String>)> = Vec::new();
        
        for (score, doc_address) in top_docs {
            match searcher.doc::<tantivy::TantivyDocument>(doc_address) {
                Ok(retrieved_doc) => {
                    let mut row_data: HashMap<String, String> = HashMap::new();
                    let mut row_id: u64 = doc_address.doc_id as u64;
                    
                    for col in &cols {
                        if let Some(field) = table_index.field_map.get(&col.column_name) {
                            if col.column_name == "_row_id" {
                                if let Some(value) = retrieved_doc.get_first(*field) {
                                    row_id = value.as_u64().unwrap_or(doc_address.doc_id as u64);
                                }
                            } else {
                                let field_entry = table_index.schema.get_field_entry(*field);
                                let field_type = field_entry.field_type();
                                
                                if let Some(value) = retrieved_doc.get_first(*field) {
                                    let s = match field_type {
                                        schema::FieldType::Str(_) => {
                                            match value.as_str() {
                                                Some(s) => s.to_string(),
                                                None => {
                                                    let bytes = value.as_bytes().unwrap_or_default();
                                                    String::from_utf8_lossy(bytes).to_string()
                                                }
                                            }
                                        }
                                        schema::FieldType::I64(_) => {
                                            format!("{}", value.as_i64().unwrap_or(0))
                                        }
                                        schema::FieldType::U64(_) => {
                                            format!("{}", value.as_u64().unwrap_or(0))
                                        }
                                        schema::FieldType::F64(_) => {
                                            format!("{}", value.as_f64().unwrap_or(0.0))
                                        }
                                        _ => {
                                            format!("{:?}", value).trim_matches('"').to_string()
                                        }
                                    };
                                    row_data.insert(col.column_name.clone(), s);
                                }
                            }
                        }
                    }
                    
                    results.push((row_id, score, row_data));
                }
                Err(e) => warn!("Failed to retrieve document {}: {}", doc_address.doc_id, e),
            }
        }
        
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use laoflchdb_engines::{ColumnType, StorageEngine};

    #[tokio::test]
    async fn test_create_table() {
        let dir = tempdir().unwrap();
        let mut engine = TantivyStorageEngine::new(dir.path().to_str().unwrap(), "test").unwrap();

        let columns = &[
            (0, "id", ColumnType::COLUMN_TYPE_INT64, None),
            (1, "name", ColumnType::COLUMN_TYPE_STRING, None),
            (2, "description", ColumnType::COLUMN_TYPE_STRING, None),
        ];

        let table_id = engine.create_table("test_table", Some("Test table"), columns).await.unwrap();
        assert_ne!(table_id, 0);

        let tables = engine.list_tables().await.unwrap();
        assert!(tables.contains(&"test_table".to_string()));
    }

    #[tokio::test]
    async fn test_add_and_get_row() {
        let dir = tempdir().unwrap();
        let mut engine = TantivyStorageEngine::new(dir.path().to_str().unwrap(), "test").unwrap();

        let columns = &[
            (0, "name", ColumnType::COLUMN_TYPE_STRING, None),
            (1, "email", ColumnType::COLUMN_TYPE_STRING, None),
        ];

        engine.create_table("users", None, columns).await.unwrap();

        let row = Row {
            row_type: EnumOrUnknown::from(RowType::ROW_TYPE_NORMAL),
            version: 1,
            data: vec![b"John".to_vec(), b"john@example.com".to_vec()],
            special_fields: SpecialFields::new(),
        };

        let row_id = engine.add_row("users", &row).await.unwrap();
        assert_ne!(row_id, 0);

        let retrieved = engine.get_row("users", row_id).await.unwrap();
        assert!(retrieved.is_some());

        let retrieved_row = retrieved.unwrap();
        assert_eq!(retrieved_row.data[0], b"John");
        assert_eq!(retrieved_row.data[1], b"john@example.com");
    }

    #[tokio::test]
    async fn test_delete_row() {
        let dir = tempdir().unwrap();
        let mut engine = TantivyStorageEngine::new(dir.path().to_str().unwrap(), "test").unwrap();

        let columns = &[(0, "name", ColumnType::COLUMN_TYPE_STRING, None)];
        engine.create_table("test_table", None, columns).await.unwrap();

        let row = Row {
            row_type: EnumOrUnknown::from(RowType::ROW_TYPE_NORMAL),
            version: 1,
            data: vec![b"Test".to_vec()],
            special_fields: SpecialFields::new(),
        };

        let row_id = engine.add_row("test_table", &row).await.unwrap();
        
        assert!(engine.get_row("test_table", row_id).await.unwrap().is_some());
        
        engine.delete_row("test_table", row_id).await.unwrap();
        
        assert!(engine.get_row("test_table", row_id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_scan_table() {
        let dir = tempdir().unwrap();
        let mut engine = TantivyStorageEngine::new(dir.path().to_str().unwrap(), "test").unwrap();

        let columns = &[(0, "name", ColumnType::COLUMN_TYPE_STRING, None)];
        engine.create_table("test_table", None, columns).await.unwrap();

        for i in 0..5 {
            let row = Row {
                row_type: EnumOrUnknown::from(RowType::ROW_TYPE_NORMAL),
                version: 1,
                data: vec![format!("Item {}", i).as_bytes().to_vec()],
                special_fields: SpecialFields::new(),
            };
            engine.add_row("test_table", &row).await.unwrap();
        }

        let results = engine.scan_table("test_table", Some(3)).await.unwrap();
        assert_eq!(results.len(), 3);

        let all_results = engine.scan_table("test_table", None).await.unwrap();
        assert_eq!(all_results.len(), 5);
    }
}