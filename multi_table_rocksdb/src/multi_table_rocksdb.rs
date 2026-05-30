use rocksdb::{DB, Options, ColumnFamilyDescriptor};
use prost::Message;
use std::sync::{Arc, RwLock};

use laoflchdb_db_engine::{DBEngine, EngineOptions, META_TABLE_PREFIX, META_COLUMN_PREFIX, META_SCHEMA_PREFIX, MAX_TABLE_ID_LENGTH};
use laoflchdb_db_engine::pb::{SchemaMeta, TableMeta, ColumnMeta, Row, ColumnType};

pub struct MultiTableRocksDBEngine {
    db: DB,
    schema_name: String,
    next_row_id: Arc<RwLock<u64>>,
}

impl MultiTableRocksDBEngine {
    pub fn new(options: &EngineOptions) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let path = &options.db_path;
        let schema_name = options.schema_name.clone();
        
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        let cf_list = DB::list_cf(&opts, path).unwrap_or_else(|_| vec!["default".to_string()]);
        
        let cf_descriptors: Vec<ColumnFamilyDescriptor> = if cf_list.is_empty() {
            vec![ColumnFamilyDescriptor::new("default", Options::default())]
        } else {
            cf_list.into_iter()
                .map(|name| ColumnFamilyDescriptor::new(name, Options::default()))
                .collect()
        };

        let db = DB::open_cf_descriptors(&opts, path, cf_descriptors)?;
        
        let mut engine = Self {
            db,
            schema_name,
            next_row_id: Arc::new(RwLock::new(1)),
        };
        
        engine.init_schema_meta()?;
        engine.init_row_id()?;
        engine.init_default_user_table()?;
        
        Ok(engine)
    }

    fn init_schema_meta(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let schema_meta_key = self.make_schema_meta_key();
        if self.get_meta_internal(schema_meta_key.as_bytes())?.is_none() {
            let schema_meta = SchemaMeta {
                schema_name: self.schema_name.clone(),
                next_auto_inc_table_id: 0,
            };
            let encoded = schema_meta.encode_to_vec();
            self.put_meta_internal(schema_meta_key.as_bytes(), &encoded)?;
        }
        Ok(())
    }

    fn init_default_user_table(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let tables = self.list_tables_internal()?;
        if !tables.contains(&"user".to_string()) {
            let next_table_id = self.get_next_table_id()?;
            
            let cf_name = self.get_table_cf("user");
            let cf_opts = Options::default();
            self.db.create_cf(&cf_name, &cf_opts)?;
            
            let table_meta_key = self.make_table_meta_key("user", next_table_id);
            let table_meta = TableMeta {
                table_id: next_table_id,
                table_name: "user".to_string(),
                column_count: 2,
                next_auto_inc_column_id: 2,
            };
            let encoded = table_meta.encode_to_vec();
            self.put_meta_internal(table_meta_key.as_bytes(), &encoded)?;
            
            let columns = vec![
                (0, "user_id", ColumnType::Int64),
                (1, "password", ColumnType::String),
            ];
            
            for (column_id, column_name, column_type) in columns {
                let col_meta_key = self.make_column_meta_key(next_table_id, column_name, column_id, column_type);
                let col_meta = ColumnMeta {
                    table_id: next_table_id,
                    column_id,
                    column_name: column_name.to_string(),
                    column_type: column_type.into(),
                };
                let encoded = col_meta.encode_to_vec();
                self.put_meta_internal(col_meta_key.as_bytes(), &encoded)?;
            }
        }
        Ok(())
    }

    fn init_row_id(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut max_id = 0u64;
        
        let all_tables = self.list_tables_internal()?;
        
        for table in &all_tables {
            let cf_name = self.get_table_cf(table);
            if let Some(cf_handle) = self.db.cf_handle(&cf_name) {
                let mut iter = self.db.raw_iterator_cf(cf_handle);
                iter.seek_to_last();
                
                while iter.valid() {
                    if let Some(key) = iter.key() {
                        if let Ok(key_str) = String::from_utf8(key.to_vec()) {
                            if let Some(id_str) = key_str.strip_prefix("row:") {
                                if let Ok(id) = id_str.parse::<u64>() {
                                    if id > max_id {
                                        max_id = id;
                                    }
                                }
                            }
                        }
                    }
                    iter.prev();
                }
            }
        }
        
        *self.next_row_id.write().unwrap() = max_id + 1;
        Ok(())
    }

    fn get_next_row_id(&self) -> u64 {
        let mut id = self.next_row_id.write().unwrap();
        let result = *id;
        *id += 1;
        result
    }

    pub fn db_path(&self) -> String {
        self.db.path().to_string_lossy().to_string()
    }

    fn get_table_cf(&self, table: &str) -> String {
        table.to_string()
    }

    fn make_schema_meta_key(&self) -> String {
        format!("{}:{}", META_SCHEMA_PREFIX, self.schema_name)
    }

    fn make_table_meta_key(&self, table_name: &str, table_id: u64) -> String {
        format!("{}:{}:{}", META_TABLE_PREFIX, table_name, table_id)
    }

    fn format_table_id(&self, table_id: u64) -> String {
        format!("{:0>width$}", table_id, width = MAX_TABLE_ID_LENGTH)
    }

    fn get_column_type_name(&self, column_type: ColumnType) -> &'static str {
        match column_type {
            ColumnType::String => "COLUMN_TYPE_STRING",
            ColumnType::Int64 => "COLUMN_TYPE_INT64",
            ColumnType::Bytes => "COLUMN_TYPE_BYTES",
            ColumnType::Float => "COLUMN_TYPE_FLOAT",
            ColumnType::List => "COLUMN_TYPE_LIST",
            ColumnType::Image => "COLUMN_TYPE_IMAGE",
        }
    }

    fn make_column_meta_key(&self, table_id: u64, column_name: &str, column_id: u64, column_type: ColumnType) -> String {
        let formatted_table_id = self.format_table_id(table_id);
        let type_name = self.get_column_type_name(column_type);
        format!("{}:{}:{}:{}:{}", META_COLUMN_PREFIX, formatted_table_id, column_name, column_id, type_name)
    }

    fn put_meta_internal(&self, key: &[u8], value: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let cf_handle = self.db.cf_handle("default")
            .ok_or_else(|| "Default column family not found".to_string())?;
        self.db.put_cf(cf_handle, key, value)?;
        Ok(())
    }

    fn get_meta_internal(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        let cf_handle = self.db.cf_handle("default")
            .ok_or_else(|| "Default column family not found".to_string())?;
        let result = self.db.get_cf(cf_handle, key)?;
        Ok(result)
    }

    fn scan_meta_prefix_internal(&self, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>, Box<dyn std::error::Error + Send + Sync>> {
        let cf_handle = self.db.cf_handle("default")
            .ok_or_else(|| "Default column family not found".to_string())?;
        
        let mut result = Vec::new();
        let mut iter = self.db.raw_iterator_cf(cf_handle);
        iter.seek(prefix);
        
        while iter.valid() {
            if let (Some(k), Some(v)) = (iter.key(), iter.value()) {
                if k.starts_with(prefix) {
                    result.push((k.to_vec(), v.to_vec()));
                    iter.next();
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        
        Ok(result)
    }

    fn delete_meta_internal(&mut self, key: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let cf_handle = self.db.cf_handle("default")
            .ok_or_else(|| "Default column family not found".to_string())?;
        self.db.delete_cf(cf_handle, key)?;
        Ok(())
    }

    fn get_next_table_id(&self) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        let schema_meta_key = self.make_schema_meta_key();
        let schema_meta_data = self.get_meta_internal(schema_meta_key.as_bytes())?;
        
        let mut next_table_id: u64 = 0;
        if let Some(data) = schema_meta_data {
            let mut schema_meta: SchemaMeta = prost::Message::decode(&data[..])?;
            next_table_id = schema_meta.next_auto_inc_table_id;
            schema_meta.next_auto_inc_table_id = next_table_id + 1;
            let encoded = schema_meta.encode_to_vec();
            self.put_meta_internal(schema_meta_key.as_bytes(), &encoded)?;
        } else {
            let schema_meta = SchemaMeta {
                schema_name: self.schema_name.clone(),
                next_auto_inc_table_id: 1,
            };
            let encoded = schema_meta.encode_to_vec();
            self.put_meta_internal(schema_meta_key.as_bytes(), &encoded)?;
        }
        
        Ok(next_table_id)
    }

    fn list_tables_internal(&self) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        let prefix = format!("{}:", META_TABLE_PREFIX);
        let entries = self.scan_meta_prefix_internal(prefix.as_bytes())?;
        
        let mut tables = std::collections::HashSet::new();
        for (key, _) in entries {
            if let Ok(key_str) = String::from_utf8(key) {
                if let Some(rest) = key_str.strip_prefix(&prefix) {
                    if let Some(table_name) = rest.split(':').next() {
                        tables.insert(table_name.to_string());
                    }
                }
            }
        }
        
        Ok(tables.into_iter().collect())
    }
}

#[async_trait::async_trait]
impl DBEngine for MultiTableRocksDBEngine {
    // 表管理
    async fn create_table(&mut self, table: &str, columns: &[(u32, &str, ColumnType)]) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        if table == "default" {
            return Err("Cannot create reserved table 'default'".into());
        }
        
        let cf_name = self.get_table_cf(table);
        
        if self.db.cf_handle(&cf_name).is_some() {
            return Err(format!("Table '{}' already exists", cf_name).into());
        }
        
        let next_table_id = self.get_next_table_id()?;
        
        let cf_opts = Options::default();
        self.db.create_cf(&cf_name, &cf_opts)?;

        let table_meta_key = self.make_table_meta_key(table, next_table_id);
        let table_meta = TableMeta {
            table_id: next_table_id,
            table_name: table.to_string(),
            column_count: columns.len() as u32,
            next_auto_inc_column_id: columns.len() as u64,
        };
        let encoded = table_meta.encode_to_vec();
        self.put_meta_internal(table_meta_key.as_bytes(), &encoded)?;

        for (idx, (_, col_name, col_type)) in columns.iter().enumerate() {
            let col_id = idx as u64;
            let col_meta_key = self.make_column_meta_key(next_table_id, col_name, col_id, *col_type);
            let col_meta = ColumnMeta {
                table_id: next_table_id,
                column_id: col_id,
                column_name: col_name.to_string(),
                column_type: (*col_type).into(),
            };
            let encoded = col_meta.encode_to_vec();
            self.put_meta_internal(col_meta_key.as_bytes(), &encoded)?;
        }

        Ok(next_table_id)
    }

    async fn drop_table(&mut self, table: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let cf_name = self.get_table_cf(table);
        
        if self.db.cf_handle(&cf_name).is_some() {
            self.db.drop_cf(&cf_name)?;
        }
        
        let prefix = format!("{}:{}:", META_TABLE_PREFIX, table);
        let entries = self.scan_meta_prefix_internal(prefix.as_bytes())?;
        
        for (key, _) in entries {
            self.delete_meta_internal(&key)?;
        }
        
        Ok(())
    }

    async fn list_tables(&self) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        let prefix = format!("{}:", META_TABLE_PREFIX);
        let entries = self.scan_meta_prefix_internal(prefix.as_bytes())?;
        
        let mut tables = std::collections::HashSet::new();
        for (key, _) in entries {
            if let Ok(key_str) = String::from_utf8(key) {
                if let Some(rest) = key_str.strip_prefix(&prefix) {
                    if let Some(table_name) = rest.split(':').next() {
                        tables.insert(table_name.to_string());
                    }
                }
            }
        }
        
        Ok(tables.into_iter().collect())
    }

    async fn list_table_cols(&self, table: &str) -> Result<Vec<ColumnMeta>, Box<dyn std::error::Error + Send + Sync>> {
        let mut cols = Vec::new();
        
        let table_prefix = format!("{}:{}:", META_TABLE_PREFIX, table);
        let table_entries = self.scan_meta_prefix_internal(table_prefix.as_bytes())?;
        
        let mut table_id = None;
        for (key, value) in table_entries {
            if let Ok(key_str) = String::from_utf8(key) {
                if let Ok(table_meta) = TableMeta::decode(value.as_slice()) {
                    table_id = Some(table_meta.table_id);
                    break;
                }
            }
        }
        
        if let Some(tid) = table_id {
            let col_prefix = format!("{}:{}:", META_COLUMN_PREFIX, self.format_table_id(tid));
            let col_entries = self.scan_meta_prefix_internal(col_prefix.as_bytes())?;
            
            for (_, value) in col_entries {
                if let Ok(col_meta) = ColumnMeta::decode(value.as_slice()) {
                    cols.push(col_meta);
                }
            }
        }
        
        cols.sort_by_key(|c| c.column_id);
        Ok(cols)
    }
    
    // 行操作
    async fn add_row(&mut self, table: &str, row: &Row) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        let cf_name = self.get_table_cf(table);
        let cf_handle = self.db.cf_handle(&cf_name)
            .ok_or_else(|| format!("Table '{}' not found", cf_name))?;
        
        let row_id = self.get_next_row_id();
        let key = format!("row:{}", row_id);
        let value = row.encode_to_vec();
        
        self.db.put_cf(cf_handle, key.as_bytes(), value)?;
        
        Ok(row_id)
    }

    async fn get_row(&self, table: &str, row_id: u64) -> Result<Option<Row>, Box<dyn std::error::Error + Send + Sync>> {
        let cf_name = self.get_table_cf(table);
        let cf_handle = self.db.cf_handle(&cf_name)
            .ok_or_else(|| format!("Table '{}' not found", cf_name))?;
        
        let key = format!("row:{}", row_id);
        let result = self.db.get_cf(cf_handle, key.as_bytes())?;
        
        match result {
            Some(data) => Ok(Some(Row::decode(data.as_slice())?)),
            None => Ok(None),
        }
    }

    async fn delete_row(&mut self, table: &str, row_id: u64) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let cf_name = self.get_table_cf(table);
        let cf_handle = self.db.cf_handle(&cf_name)
            .ok_or_else(|| format!("Table '{}' not found", cf_name))?;
        
        let key = format!("row:{}", row_id);
        self.db.delete_cf(cf_handle, key.as_bytes())?;
        
        Ok(())
    }

    async fn update_row(&mut self, table: &str, row_id: u64, row: &Row) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let cf_name = self.get_table_cf(table);
        let cf_handle = self.db.cf_handle(&cf_name)
            .ok_or_else(|| format!("Table '{}' not found", cf_name))?;
        
        let key = format!("row:{}", row_id);
        
        if self.db.get_cf(cf_handle, key.as_bytes())?.is_none() {
            return Err(format!("Row {} not found in table '{}'", row_id, table).into());
        }
        
        let value = row.encode_to_vec();
        self.db.put_cf(cf_handle, key.as_bytes(), value)?;
        
        Ok(())
    }
    
    // 元数据查询
    async fn get_all_meta(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let mut result = serde_json::Map::new();
        
        let mut tables = serde_json::Map::new();
        
        let table_entries = self.scan_meta_prefix_internal(META_TABLE_PREFIX.as_bytes())?;
        
        for (key, value) in table_entries {
            if let Ok(key_str) = String::from_utf8(key) {
                if let Ok(table_meta) = TableMeta::decode(value.as_slice()) {
                    let mut table_obj = serde_json::Map::new();
                    table_obj.insert("table_id".to_string(), table_meta.table_id.into());
                    table_obj.insert("table_name".to_string(), table_meta.table_name.clone().into());
                    table_obj.insert("column_count".to_string(), table_meta.column_count.into());
                    
                    tables.insert(table_meta.table_name, table_obj.into());
                }
            }
        }
        
        result.insert("tables".to_string(), tables.into());
        
        Ok(serde_json::to_string(&result)?)
    }

    async fn get_schema_info(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let mut result = serde_json::Map::new();
        
        let tables = self.list_tables().await?;
        let mut table_names = Vec::new();
        
        for table in &tables {
            table_names.push(table.clone());
        }
        
        result.insert("schema_name".to_string(), self.schema_name.clone().into());
        result.insert("table_count".to_string(), tables.len().into());
        result.insert("tables".to_string(), table_names.into());
        
        Ok(serde_json::to_string(&result)?)
    }

    async fn get_table_meta(&self, table: &str) -> Result<Option<TableMeta>, Box<dyn std::error::Error + Send + Sync>> {
        let prefix = format!("{}:{}:", META_TABLE_PREFIX, table);
        let meta_entries = self.scan_meta_prefix_internal(prefix.as_bytes())?;
        
        for (_, value) in meta_entries {
            let table_meta: TableMeta = prost::Message::decode(&value[..])?;
            return Ok(Some(table_meta));
        }
        
        Ok(None)
    }
    
    // KV 操作
    async fn put(&mut self, table: &str, key: &[u8], value: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let cf_name = self.get_table_cf(table);
        let cf_handle = self.db.cf_handle(&cf_name)
            .ok_or_else(|| format!("Table '{}' not found", cf_name))?;
        self.db.put_cf(cf_handle, key, value)?;
        Ok(())
    }

    async fn get(&self, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        let cf_name = self.get_table_cf(table);
        let cf_handle = self.db.cf_handle(&cf_name)
            .ok_or_else(|| format!("Table '{}' not found", cf_name))?;
        let result = self.db.get_cf(cf_handle, key)?;
        Ok(result)
    }

    async fn delete(&mut self, table: &str, key: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let cf_name = self.get_table_cf(table);
        let cf_handle = self.db.cf_handle(&cf_name)
            .ok_or_else(|| format!("Table '{}' not found", cf_name))?;
        self.db.delete_cf(cf_handle, key)?;
        Ok(())
    }

    fn get_schema_name(&self) -> &str {
        &self.schema_name
    }
}
