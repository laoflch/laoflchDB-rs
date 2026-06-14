use std::collections::HashMap;
use std::sync::Mutex;
use std::path::Path;

use async_trait::async_trait;
use log::info;
use tantivy::{
    directory::MmapDirectory,
    schema::{Field, Schema, TextOptions, TextFieldIndexing, IndexRecordOption, NumericOptions, STORED},
    Index, IndexReader, IndexWriter, IndexSettings,
};
use uuid::Uuid;

use laoflchdb_engines::{
    ColumnMeta, ColumnType, Query, QueryResult, Row,
    StorageEngine, TableMeta, SpecialFields,
};

/// Tantivy-based full-text search storage engine
pub struct TantivyStorageEngine {
    index_path: String,
    schema_name: String,
    index: Mutex<Option<Index>>,
    writer: Mutex<Option<IndexWriter>>,
    reader: Mutex<Option<IndexReader>>,
    tables: Mutex<HashMap<String, TableMeta>>,
    field_mappings: Mutex<HashMap<String, HashMap<String, Field>>>,
    tantivy_schemas: Mutex<HashMap<String, Schema>>,
}

impl TantivyStorageEngine {
    pub fn new(index_path: &str, schema_name: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let path = Path::new(index_path);
        if !path.exists() {
            std::fs::create_dir_all(path)?;
        }

        Ok(Self {
            index_path: index_path.to_string(),
            schema_name: schema_name.to_string(),
            index: Mutex::new(None),
            writer: Mutex::new(None),
            reader: Mutex::new(None),
            tables: Mutex::new(HashMap::new()),
            field_mappings: Mutex::new(HashMap::new()),
            tantivy_schemas: Mutex::new(HashMap::new()),
        })
    }

    fn init_index(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // 先检查是否已经初始化，避免不必要的锁定
        {
            let index_guard = self.index.lock().map_err(|e| format!("Failed to lock index: {}", e))?;
            if index_guard.is_some() {
                return Ok(());
            }
        } // 在这里释放锁

        let path = Path::new(&self.index_path);
        let directory: MmapDirectory = MmapDirectory::open(path)?;

        let index = if Index::exists(&directory)? {
            info!("Loading existing index from {}", self.index_path);
            Index::open(directory)?
        } else {
            info!("Creating new index at {}", self.index_path);
            let default_schema = Schema::builder().build();
            Index::create(directory, default_schema, IndexSettings::default())?
        };

        // 再次获取锁来设置 index
        {
            let mut index_guard = self.index.lock().map_err(|e| format!("Failed to lock index: {}", e))?;
            *index_guard = Some(index);
        }

        self.init_writer()?;
        self.init_reader()?;

        Ok(())
    }

    fn init_writer(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let index_guard = self.index.lock().map_err(|e| format!("Failed to lock index: {}", e))?;
        let index = index_guard.as_ref().ok_or("Index not initialized")?;

        let mut writer_guard = self.writer.lock().map_err(|e| format!("Failed to lock writer: {}", e))?;
        *writer_guard = Some(index.writer(50_000_000)?);

        Ok(())
    }

    fn init_reader(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let index_guard = self.index.lock().map_err(|e| format!("Failed to lock index: {}", e))?;
        let index = index_guard.as_ref().ok_or("Index not initialized")?;

        let mut reader_guard = self.reader.lock().map_err(|e| format!("Failed to lock reader: {}", e))?;
        *reader_guard = Some(index.reader_builder().try_into()?);

        Ok(())
    }

    fn create_tantivy_schema(&self, columns: &[(u32, &str, ColumnType, Option<&str>)]) -> Schema {
        let mut schema_builder = Schema::builder();

        for (_idx, name, col_type, _comment) in columns {
            match col_type {
                ColumnType::COLUMN_TYPE_STRING | ColumnType::COLUMN_TYPE_BYTES => {
                    let text_options = TextOptions::default()
                        .set_stored()
                        .set_indexing_options(
                            TextFieldIndexing::default()
                                .set_tokenizer("default")
                                .set_index_option(IndexRecordOption::WithFreqsAndPositions)
                        );
                    schema_builder.add_text_field(name, text_options);
                }
                ColumnType::COLUMN_TYPE_INT64 => {
                    let numeric_options = NumericOptions::default()
                        .set_stored()
                        .set_indexed();
                    schema_builder.add_i64_field(name, numeric_options);
                }
                ColumnType::COLUMN_TYPE_FLOAT => {
                    let numeric_options = NumericOptions::default()
                        .set_stored()
                        .set_indexed();
                    schema_builder.add_f64_field(name, numeric_options);
                }
                _ => {
                    schema_builder.add_text_field(name, STORED);
                }
            }
        }

        schema_builder.build()
    }
}

#[async_trait]
impl StorageEngine for TantivyStorageEngine {
    async fn create_table(&mut self, table: &str, table_comment: Option<&str>, columns: &[(u32, &str, ColumnType, Option<&str>)]) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        self.init_index()?;

        let table_id = Uuid::new_v4().as_u128() as u64;
        let column_count = columns.len() as u32;

        let tantivy_schema = self.create_tantivy_schema(columns);

        let mut field_map: HashMap<String, Field> = HashMap::new();
        for (_idx, name, _col_type, _comment) in columns {
            match tantivy_schema.get_field(name) {
                Ok(field) => {
                    field_map.insert(name.to_string(), field);
                }
                Err(_) => {}
            }
        }

        let table_meta = TableMeta {
            table_id,
            table_name: table.to_string(),
            column_count,
            comment: table_comment.unwrap_or("").to_string(),
            next_auto_inc_column_id: columns.len() as u64,
            special_fields: SpecialFields::new(),
        };

        let mut tables_guard = self.tables.lock().map_err(|e| format!("Failed to lock tables: {}", e))?;
        tables_guard.insert(table.to_string(), table_meta);

        let mut field_mappings_guard = self.field_mappings.lock().map_err(|e| format!("Failed to lock field mappings: {}", e))?;
        field_mappings_guard.insert(table.to_string(), field_map);

        let mut tantivy_schemas_guard = self.tantivy_schemas.lock().map_err(|e| format!("Failed to lock tantivy schemas: {}", e))?;
        tantivy_schemas_guard.insert(table.to_string(), tantivy_schema);

        info!("Created table '{}' with {} columns in schema '{}'", table, column_count, self.schema_name);
        Ok(table_id)
    }

    async fn drop_table(&mut self, table: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut tables_guard = self.tables.lock().map_err(|e| format!("Failed to lock tables: {}", e))?;
        let mut field_mappings_guard = self.field_mappings.lock().map_err(|e| format!("Failed to lock field mappings: {}", e))?;
        let mut tantivy_schemas_guard = self.tantivy_schemas.lock().map_err(|e| format!("Failed to lock tantivy schemas: {}", e))?;

        tables_guard.remove(table);
        field_mappings_guard.remove(table);
        tantivy_schemas_guard.remove(table);

        info!("Dropped table '{}' from schema '{}'", table, self.schema_name);
        Ok(())
    }

    async fn list_tables(&self) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        let tables_guard = self.tables.lock().map_err(|e| format!("Failed to lock tables: {}", e))?;
        Ok(tables_guard.keys().cloned().collect())
    }

    async fn list_table_cols(&self, table: &str) -> Result<Vec<ColumnMeta>, Box<dyn std::error::Error + Send + Sync>> {
        let tables_guard = self.tables.lock().map_err(|e| format!("Failed to lock tables: {}", e))?;
        let table_meta = tables_guard.get(table).ok_or(format!("Table '{}' not found", table))?;

        let field_mappings_guard = self.field_mappings.lock().map_err(|e| format!("Failed to lock field mappings: {}", e))?;
        let field_map = field_mappings_guard.get(table).ok_or(format!("Field mappings not found for table '{}'", table))?;

        let mut columns: Vec<ColumnMeta> = Vec::new();
        for (idx, (name, _field)) in field_map.iter().enumerate() {
            columns.push(ColumnMeta {
                column_id: idx as u64,
                column_name: name.clone(),
                column_type: ColumnType::COLUMN_TYPE_STRING.into(),
                comment: "".to_string(),
                special_fields: SpecialFields::new(),
                table_id: table_meta.table_id,
            });
        }

        Ok(columns)
    }

    async fn add_row(&mut self, _table: &str, _row: &Row) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        todo!("add_row not implemented for TantivyStorageEngine")
    }

    async fn get_row(&self, _table: &str, _row_id: u64) -> Result<Option<Row>, Box<dyn std::error::Error + Send + Sync>> {
        todo!("get_row not implemented for TantivyStorageEngine")
    }

    async fn delete_row(&mut self, _table: &str, _row_id: u64) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        todo!("delete_row not implemented for TantivyStorageEngine")
    }

    async fn update_row(&mut self, _table: &str, _row_id: u64, _row: &Row) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        todo!("update_row not implemented for TantivyStorageEngine")
    }

    async fn get_all_meta(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let tables_guard = self.tables.lock().map_err(|e| format!("Failed to lock tables: {}", e))?;
        let mut result = format!("Schema: {}\n", self.schema_name);
        result.push_str(&format!("Tables count: {}\n", tables_guard.len()));
        for (name, meta) in tables_guard.iter() {
            result.push_str(&format!("  - {} (id: {}, columns: {})\n", name, meta.table_id, meta.column_count));
        }
        Ok(result)
    }

    async fn get_schema_info(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok(format!("Schema: {}, Path: {}", self.schema_name, self.index_path))
    }

    async fn get_table_meta(&self, table: &str) -> Result<Option<TableMeta>, Box<dyn std::error::Error + Send + Sync>> {
        let tables_guard = self.tables.lock().map_err(|e| format!("Failed to lock tables: {}", e))?;
        Ok(tables_guard.get(table).cloned())
    }

    async fn update_table_comment(&mut self, table: &str, comment: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut tables_guard = self.tables.lock().map_err(|e| format!("Failed to lock tables: {}", e))?;
        if let Some(meta) = tables_guard.get_mut(table) {
            meta.comment = comment.to_string();
            Ok(())
        } else {
            Err(format!("Table '{}' not found", table).into())
        }
    }

    async fn update_column_comment(&mut self, _table: &str, _column_name: &str, _comment: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        todo!("update_column_comment not implemented for TantivyStorageEngine")
    }

    async fn put(&mut self, _table: &str, _key: &[u8], _value: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        todo!("put not implemented for TantivyStorageEngine")
    }

    async fn get(&self, _table: &str, _key: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        todo!("get not implemented for TantivyStorageEngine")
    }

    async fn delete(&mut self, _table: &str, _key: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        todo!("delete not implemented for TantivyStorageEngine")
    }

    async fn query(&self, _query: &Query) -> Result<QueryResult, Box<dyn std::error::Error + Send + Sync>> {
        todo!("query not implemented for TantivyStorageEngine")
    }

    fn get_schema_name(&self) -> &str {
        &self.schema_name
    }

    async fn scan_table(&self, _table: &str, _limit: Option<usize>) -> Result<Vec<(u64, Row)>, Box<dyn std::error::Error + Send + Sync>> {
        todo!("scan_table not implemented for TantivyStorageEngine")
    }

    async fn get_column_types(&self, table: &str) -> Result<std::collections::HashMap<String, ColumnType>, Box<dyn std::error::Error + Send + Sync>> {
        let field_mappings_guard = self.field_mappings.lock().map_err(|e| format!("Failed to lock field mappings: {}", e))?;
        let field_map = field_mappings_guard.get(table).ok_or(format!("Table '{}' not found", table))?;

        let mut result = std::collections::HashMap::new();
        for name in field_map.keys() {
            result.insert(name.clone(), ColumnType::COLUMN_TYPE_STRING);
        }
        Ok(result)
    }
}