use log::warn;
use rocksdb::{DB, Options, ColumnFamilyDescriptor};
use protobuf::{Message, Enum};
use snowflake_me::Snowflake;
use std::sync::Arc;
use std::sync::RwLock;
use tokio::sync::RwLock as TokioRwLock;

use laoflchdb_engines::{StorageEngine, EngineOptions, META_TABLE_PREFIX, META_COLUMN_PREFIX, META_SCHEMA_PREFIX, MAX_TABLE_ID_LENGTH};
use laoflchdb_sql_df_engine::DataFusionStorageEngine;
use laoflchdb_engines::{SchemaMeta, TableMeta, ColumnMeta, Row, ColumnType, Query, QueryResult, QueryRow,
                                  FilterOperator, ColumnFilter, ColumnFilterCondition, TableFilter};

use datafusion::arrow::array::{ArrayRef, StringArray, Int64Array, Float64Array, BinaryArray};
use datafusion::arrow::datatypes::{DataType, Field as ArrowField, Schema};
use datafusion::catalog::Session;
use datafusion::datasource::TableProvider;
use datafusion::physical_plan::ExecutionPlan;

use crate::rocksdb_table::{RocksDBTable, FilterGroup, FilterItem, FilterRelation};

fn write_proto_to_vec<T: Message>(msg: &T) -> Vec<u8> {
    let mut v = Vec::new();
    msg.write_to_vec(&mut v).expect("Failed to serialize protobuf");
    v
}

fn parse_proto_from_bytes<T: Message + Default>(bytes: &[u8]) -> Result<T, protobuf::Error> {
    T::parse_from_bytes(bytes)
}

#[derive(Clone)]
pub struct MultiTableRocksDBEngine {
    db: Arc<RwLock<DB>>,
    schema_name: String,
    snowflake: Snowflake,
}

impl std::fmt::Debug for MultiTableRocksDBEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiTableRocksDBEngine")
            .field("schema_name", &self.schema_name)
            .finish()
    }
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
        let db = Arc::new(RwLock::new(db));
        
        let snowflake = Snowflake::new()?;
        
        let mut engine = Self {
            db,
            schema_name,
            snowflake,
        };
        
        engine.init_schema_meta()?;
        engine.init_default_user_table()?;
        
        Ok(engine)
    }

    fn init_schema_meta(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let schema_meta_key = self.make_schema_meta_key();
        if self.get_meta_internal(schema_meta_key.as_bytes())?.is_none() {
            let schema_meta = SchemaMeta {
                schema_name: self.schema_name.clone(),
                next_auto_inc_table_id: 0,
                special_fields: ::protobuf::SpecialFields::default(),
            };
            let encoded = write_proto_to_vec(&schema_meta);
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
            self.db.write().unwrap().create_cf(&cf_name, &cf_opts)?;
            
            let table_meta_key = self.make_table_meta_key("user", next_table_id);
            let table_meta = TableMeta {
                table_id: next_table_id,
                table_name: "user".to_string(),
                column_count: 2,
                next_auto_inc_column_id: 2,
                special_fields: ::protobuf::SpecialFields::default(),
            };
            let encoded = write_proto_to_vec(&table_meta);
            self.put_meta_internal(table_meta_key.as_bytes(), &encoded)?;
            
            let columns = vec![
                (0, "user_id", ColumnType::COLUMN_TYPE_INT64),
                (1, "password", ColumnType::COLUMN_TYPE_STRING),
            ];
            
            for (column_id, column_name, column_type) in columns {
                let col_meta_key = self.make_column_meta_key(next_table_id, column_name, column_id, column_type);
                let col_meta = ColumnMeta {
                    table_id: next_table_id,
                    column_id,
                    column_name: column_name.to_string(),
                    column_type: column_type.into(),
                    special_fields: ::protobuf::SpecialFields::default(),
                };
                let encoded = write_proto_to_vec(&col_meta);
                self.put_meta_internal(col_meta_key.as_bytes(), &encoded)?;
            }
        }
        Ok(())
    }

    fn get_next_row_id(&self) -> u64 {
        self.snowflake.next_id().unwrap_or_else(|_| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64
        })
    }

    pub fn db_path(&self) -> String {
        self.db.read().unwrap().path().to_string_lossy().to_string()
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
            ColumnType::COLUMN_TYPE_STRING => "COLUMN_TYPE_STRING",
            ColumnType::COLUMN_TYPE_INT64 => "COLUMN_TYPE_INT64",
            ColumnType::COLUMN_TYPE_BYTES => "COLUMN_TYPE_BYTES",
            ColumnType::COLUMN_TYPE_FLOAT => "COLUMN_TYPE_FLOAT",
            ColumnType::COLUMN_TYPE_LIST => "COLUMN_TYPE_LIST",
            ColumnType::COLUMN_TYPE_IMAGE => "COLUMN_TYPE_IMAGE",
            _ => "COLUMN_TYPE_STRING",
        }
    }

    fn make_column_meta_key(&self, table_id: u64, column_name: &str, column_id: u64, column_type: ColumnType) -> String {
        let formatted_table_id = self.format_table_id(table_id);
        let type_name = self.get_column_type_name(column_type);
        format!("{}:{}:{}:{}:{}", META_COLUMN_PREFIX, formatted_table_id, column_name, column_id, type_name)
    }

    fn put_meta_internal(&self, key: &[u8], value: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let db = self.db.read().unwrap();
        let cf_handle = db.cf_handle("default")
            .ok_or_else(|| "Default column family not found".to_string())?;
        db.put_cf(cf_handle, key, value)?;
        Ok(())
    }

    fn get_meta_internal(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        let db = self.db.read().unwrap();
        let cf_handle = db.cf_handle("default")
            .ok_or_else(|| "Default column family not found".to_string())?;
        let result = db.get_cf(cf_handle, key)?;
        Ok(result)
    }

    fn scan_meta_prefix_internal(&self, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>, Box<dyn std::error::Error + Send + Sync>> {
        let db = self.db.read().unwrap();
        let cf_handle = db.cf_handle("default")
            .ok_or_else(|| "Default column family not found".to_string())?;
        
        let mut result = Vec::new();
        let mut iter = db.raw_iterator_cf(cf_handle);
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
        let db = self.db.read().unwrap();
        let cf_handle = db.cf_handle("default")
            .ok_or_else(|| "Default column family not found".to_string())?;
        db.delete_cf(cf_handle, key)?;
        Ok(())
    }

    fn get_next_table_id(&self) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        let schema_meta_key = self.make_schema_meta_key();
        let schema_meta_data = self.get_meta_internal(schema_meta_key.as_bytes())?;
        
        let mut next_table_id: u64 = 0;
        if let Some(data) = schema_meta_data {
            let mut schema_meta: SchemaMeta = parse_proto_from_bytes(&data[..])?;
            next_table_id = schema_meta.next_auto_inc_table_id;
            schema_meta.next_auto_inc_table_id = next_table_id + 1;
            let encoded = write_proto_to_vec(&schema_meta);
            self.put_meta_internal(schema_meta_key.as_bytes(), &encoded)?;
        } else {
            let schema_meta = SchemaMeta {
                schema_name: self.schema_name.clone(),
                next_auto_inc_table_id: 1,
                special_fields: ::protobuf::SpecialFields::default(),
            };
            let encoded = write_proto_to_vec(&schema_meta);
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
impl StorageEngine for MultiTableRocksDBEngine {
    async fn create_table(&mut self, table: &str, columns: &[(u32, &str, ColumnType)]) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        if table == "default" {
            return Err("Cannot create reserved table 'default'".into());
        }
        
        let cf_name = self.get_table_cf(table);
        
        {
            let db = self.db.read().unwrap();
            if db.cf_handle(&cf_name).is_some() {
                return Err(format!("Table '{}' already exists", cf_name).into());
            }
        }
        
        let next_table_id = self.get_next_table_id()?;
        
        let cf_opts = Options::default();
        self.db.write().unwrap().create_cf(&cf_name, &cf_opts)?;

        let table_meta_key = self.make_table_meta_key(table, next_table_id);
        let table_meta = TableMeta {
            table_id: next_table_id,
            table_name: table.to_string(),
            column_count: columns.len() as u32,
            next_auto_inc_column_id: columns.len() as u64,
            special_fields: ::protobuf::SpecialFields::default(),
        };
        let encoded = write_proto_to_vec(&table_meta);
        self.put_meta_internal(table_meta_key.as_bytes(), &encoded)?;

        for (idx, (_, col_name, col_type)) in columns.iter().enumerate() {
            let col_id = idx as u64;
            let col_meta_key = self.make_column_meta_key(next_table_id, col_name, col_id, *col_type);
            let col_meta = ColumnMeta {
                table_id: next_table_id,
                column_id: col_id,
                column_name: col_name.to_string(),
                column_type: (*col_type).into(),
                special_fields: ::protobuf::SpecialFields::default(),
            };
            let encoded = write_proto_to_vec(&col_meta);
            self.put_meta_internal(col_meta_key.as_bytes(), &encoded)?;
        }

        Ok(next_table_id)
    }

    async fn drop_table(&mut self, table: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let cf_name = self.get_table_cf(table);
        
        {
            let db = self.db.read().unwrap();
            if db.cf_handle(&cf_name).is_some() {
                drop(db);
                self.db.write().unwrap().drop_cf(&cf_name)?;
            }
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
        for (_, value) in table_entries {
            if let Ok(table_meta) = TableMeta::parse_from_bytes(value.as_slice()) {
                table_id = Some(table_meta.table_id);
                break;
            }
        }
        
        if let Some(tid) = table_id {
            let col_prefix = format!("{}:{}:", META_COLUMN_PREFIX, self.format_table_id(tid));
            let col_entries = self.scan_meta_prefix_internal(col_prefix.as_bytes())?;
            
            for (_, value) in col_entries {
                if let Ok(col_meta) = ColumnMeta::parse_from_bytes(value.as_slice()) {
                    cols.push(col_meta);
                }
            }
        }
        
        cols.sort_by_key(|c| c.column_id);
        Ok(cols)
    }
    
    async fn add_row(&mut self, table: &str, row: &Row) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        let cf_name = self.get_table_cf(table);
        let db = self.db.read().unwrap();
        let cf_handle = db.cf_handle(&cf_name)
            .ok_or_else(|| format!("Table '{}' not found", cf_name))?;
        
        let row_id = self.get_next_row_id();
        let key = self.row_id_to_key(row_id);
        let value = write_proto_to_vec(row);
        
        db.put_cf(cf_handle, &key, value)?;
        
        Ok(row_id)
    }

    async fn get_row(&self, table: &str, row_id: u64) -> Result<Option<Row>, Box<dyn std::error::Error + Send + Sync>> {
        let cf_name = self.get_table_cf(table);
        let db = self.db.read().unwrap();
        let cf_handle = db.cf_handle(&cf_name)
            .ok_or_else(|| format!("Table '{}' not found", cf_name))?;
        
        let key = self.row_id_to_key(row_id);
        let result = db.get_cf(cf_handle, &key)?;
        
        match result {
            Some(data) => Ok(Some(Row::parse_from_bytes(data.as_slice())?)),
            None => Ok(None),
        }
    }

    async fn delete_row(&mut self, table: &str, row_id: u64) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let cf_name = self.get_table_cf(table);
        let db = self.db.read().unwrap();
        let cf_handle = db.cf_handle(&cf_name)
            .ok_or_else(|| format!("Table '{}' not found", cf_name))?;
        
        let key = self.row_id_to_key(row_id);
        db.delete_cf(cf_handle, &key)?;
        
        Ok(())
    }

    async fn update_row(&mut self, table: &str, row_id: u64, row: &Row) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let cf_name = self.get_table_cf(table);
        let db = self.db.read().unwrap();
        let cf_handle = db.cf_handle(&cf_name)
            .ok_or_else(|| format!("Table '{}' not found", cf_name))?;
        
        let key = self.row_id_to_key(row_id);
        
        if db.get_cf(cf_handle, &key)?.is_none() {
            return Err(format!("Row {} not found in table '{}'", row_id, table).into());
        }
        
        let value = write_proto_to_vec(row);
        db.put_cf(cf_handle, &key, value)?;
        
        Ok(())
    }
    
    async fn get_all_meta(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let mut result = serde_json::Map::new();
        
        let mut tables = serde_json::Map::new();
        
        let table_entries = self.scan_meta_prefix_internal(META_TABLE_PREFIX.as_bytes())?;
        
        for (_, value) in table_entries {
            if let Ok(table_meta) = TableMeta::parse_from_bytes(value.as_slice()) {
                let mut table_obj = serde_json::Map::new();
                table_obj.insert("table_id".to_string(), table_meta.table_id.into());
                table_obj.insert("table_name".to_string(), table_meta.table_name.clone().into());
                table_obj.insert("column_count".to_string(), table_meta.column_count.into());
                
                tables.insert(table_meta.table_name, table_obj.into());
            }
        }
        
        result.insert("tables".to_string(), tables.into());
        
        Ok(serde_json::to_string(&result)?)
    }

    async fn get_schema_info(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let mut result = serde_json::Map::new();
        
        let tables = StorageEngine::list_tables(self).await?;
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
            let table_meta: TableMeta = TableMeta::parse_from_bytes(&value[..])?;
            return Ok(Some(table_meta));
        }
        
        Ok(None)
    }
    
    async fn put(&mut self, table: &str, key: &[u8], value: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let cf_name = self.get_table_cf(table);
        let db = self.db.read().unwrap();
        let cf_handle = db.cf_handle(&cf_name)
            .ok_or_else(|| format!("Table '{}' not found", cf_name))?;
        db.put_cf(cf_handle, key, value)?;
        Ok(())
    }

    async fn get(&self, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        let cf_name = self.get_table_cf(table);
        let db = self.db.read().unwrap();
        let cf_handle = db.cf_handle(&cf_name)
            .ok_or_else(|| format!("Table '{}' not found", cf_name))?;
        let result = db.get_cf(cf_handle, key)?;
        Ok(result)
    }

    async fn delete(&mut self, table: &str, key: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let cf_name = self.get_table_cf(table);
        let db = self.db.read().unwrap();
        let cf_handle = db.cf_handle(&cf_name)
            .ok_or_else(|| format!("Table '{}' not found", cf_name))?;
        db.delete_cf(cf_handle, key)?;
        Ok(())
    }

    async fn query(&self, query: &Query) -> Result<QueryResult, Box<dyn std::error::Error + Send + Sync>> {
        let mut result_rows = Vec::new();

        for table_filter in &query.table_filters {
            self.query_table(table_filter, &mut result_rows)?;
        }

        let start = query.offset.unwrap_or(0) as usize;
        let count = query.limit.map(|l| l as usize).unwrap_or(usize::MAX);
        let end = std::cmp::min(start + count, result_rows.len());
        let rows = result_rows.drain(start..end).collect();

        Ok(QueryResult { 
            rows,
            columns: Vec::new(),
            special_fields: ::protobuf::SpecialFields::default(),
        })
    }

    fn get_schema_name(&self) -> &str {
        &self.schema_name
    }
    
    async fn scan_table(&self, table: &str, limit: Option<usize>) -> Result<Vec<(u64, Row)>, Box<dyn std::error::Error + Send + Sync>> {
        let cf_name = self.get_table_cf(table);
        let db = self.db.read().unwrap();
        let cf_handle = db.cf_handle(&cf_name)
            .ok_or_else(|| format!("Table '{}' not found", cf_name))?;
        
        let mut results = Vec::new();
        let iter = db.iterator_cf(cf_handle, rocksdb::IteratorMode::Start);
        
        for item in iter {
            let (key, value) = item?;
            let row_id = self.key_to_row_id(key.as_ref())?;
            let row = Row::parse_from_bytes(&value[..])?;
            results.push((row_id, row));
            
            if let Some(lim) = limit {
                if results.len() >= lim {
                    break;
                }
            }
        }
        
        Ok(results)
    }
    
    async fn get_column_types(&self, table: &str) -> Result<std::collections::HashMap<String, ColumnType>, Box<dyn std::error::Error + Send + Sync>> {
        let columns = self.get_table_columns(table)?;
        let mut column_types = std::collections::HashMap::new();
        
        for (name, (_, col_type)) in columns {
            column_types.insert(name, col_type);
        }
        
        Ok(column_types)
    }
}

impl MultiTableRocksDBEngine {
    fn row_id_to_key(&self, row_id: u64) -> Vec<u8> {
        row_id.to_be_bytes().to_vec()
    }
    
    fn key_to_row_id(&self, key: &[u8]) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        let key_bytes = if key.len() == 9 {
            &key[1..]
        } else if key.len() == 8 {
            key
        } else {
            return Err(format!("Invalid row key length: {}", key.len()).into());
        };
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(key_bytes);
        Ok(u64::from_be_bytes(bytes))
    }
}

impl MultiTableRocksDBEngine {
    fn get_table_columns(&self, table_name: &str) -> Result<std::collections::HashMap<String, (u64, ColumnType)>, Box<dyn std::error::Error + Send + Sync>> {
        let mut columns = std::collections::HashMap::new();
        
        let table_prefix = format!("{}:{}:", META_TABLE_PREFIX, table_name);
        let table_entries = self.scan_meta_prefix_internal(table_prefix.as_bytes())?;
        
        let mut table_id = None;
        for (_, value) in table_entries {
            if let Ok(table_meta) = TableMeta::parse_from_bytes(value.as_slice()) {
                table_id = Some(table_meta.table_id);
                break;
            }
        }
        
        if let Some(tid) = table_id {
            let col_prefix = format!("{}:{}:", META_COLUMN_PREFIX, self.format_table_id(tid));
            let col_entries = self.scan_meta_prefix_internal(col_prefix.as_bytes())?;
            
            for (_, value) in col_entries {
                if let Ok(col_meta) = ColumnMeta::parse_from_bytes(value.as_slice()) {
                    columns.insert(
                        col_meta.column_name.clone(), 
                        (col_meta.column_id, col_meta.column_type.enum_value_or(ColumnType::COLUMN_TYPE_STRING))
                    );
                }
            }
        }
        
        Ok(columns)
    }

    fn query_table(&self, table_filter: &TableFilter, result_rows: &mut Vec<QueryRow>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let table_name = &table_filter.table_name;
        let cf_name = self.get_table_cf(table_name);
        let db = self.db.read().unwrap();
        let cf_handle = match db.cf_handle(&cf_name) {
            Some(handle) => handle,
            None => return Ok(()),
        };

        let columns = self.get_table_columns(table_name)?;

        let iter = db.iterator_cf(cf_handle, rocksdb::IteratorMode::Start);
        for item in iter {
            let (key, value) = item?;

            let row_id = match self.key_to_row_id(key.as_ref()) {
                Ok(id) => id,
                Err(_) => continue,
            };

            let row = match Row::parse_from_bytes(&value[..]) {
                Ok(row) => row,
                Err(_) => continue,
            };

            if self.check_table_filters(&row, &table_filter.column_filters, &columns) {
                result_rows.push(QueryRow {
                    table_name: table_name.to_string(),
                    row_id,
                    row: ::protobuf::MessageField::some(row),
                    special_fields: ::protobuf::SpecialFields::default(),
                });
            }
        }

        Ok(())
    }

    fn query_table_negated(&self, table_filter: &TableFilter, result_rows: &mut Vec<QueryRow>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let table_name = &table_filter.table_name;
        let cf_name = self.get_table_cf(table_name);
        let db = self.db.read().unwrap();
        let cf_handle = match db.cf_handle(&cf_name) {
            Some(handle) => handle,
            None => return Ok(()),
        };

        let columns = self.get_table_columns(table_name)?;

        let iter = db.iterator_cf(cf_handle, rocksdb::IteratorMode::Start);
        for item in iter {
            let (key, value) = item?;

            let row_id = match self.key_to_row_id(key.as_ref()) {
                Ok(id) => id,
                Err(_) => continue,
            };

            let row = match Row::parse_from_bytes(&value[..]) {
                Ok(row) => row,
                Err(_) => continue,
            };

            // 取反模式：获取不满足过滤器条件的行
            if !self.check_table_filters(&row, &table_filter.column_filters, &columns) {
                result_rows.push(QueryRow {
                    table_name: table_name.to_string(),
                    row_id,
                    row: ::protobuf::MessageField::some(row),
                    special_fields: ::protobuf::SpecialFields::default(),
                });
            }
        }

        Ok(())
    }

    fn check_table_filters(
        &self, 
        row: &Row, 
        column_filters: &[ColumnFilter], 
        columns: &std::collections::HashMap<String, (u64, ColumnType)>
    ) -> bool {
        for column_filter in column_filters {
            if !self.check_column_filter(row, column_filter, columns) {
                return false;
            }
        }
        true
    }

    fn check_column_filter(
        &self, 
        row: &Row, 
        column_filter: &ColumnFilter, 
        columns: &std::collections::HashMap<String, (u64, ColumnType)>
    ) -> bool {
        for condition in &column_filter.conditions {
            if self.check_column_condition(row, column_filter, condition, columns) {
                return true;
            }
        }
        false
    }

    fn check_column_condition(
        &self, 
        row: &Row, 
        column_filter: &ColumnFilter, 
        condition: &ColumnFilterCondition, 
        columns: &std::collections::HashMap<String, (u64, ColumnType)>
    ) -> bool {
        let (column_idx, column_type) = match columns.get(&column_filter.column_name) {
            Some((idx, t)) => (*idx as usize, *t),
            None => return false,
        };

        let field_bytes = if column_idx < row.data.len() {
            &row.data[column_idx]
        } else {
            return false;
        };

        let op_val = condition.op.value();
        let op = FilterOperator::from_i32(op_val);
        match op {
            Some(FilterOperator::FILTER_OPERATOR_EQ) => {
                if let Some(value) = condition.value.as_ref() {
                    self.compare_field_equals(field_bytes, value, column_type)
                } else {
                    false
                }
            }
            Some(FilterOperator::FILTER_OPERATOR_NEQ) => {
                if let Some(value) = condition.value.as_ref() {
                    !self.compare_field_equals(field_bytes, value, column_type)
                } else {
                    false
                }
            }
            Some(FilterOperator::FILTER_OPERATOR_GT) => {
                if let Some(value) = condition.value.as_ref() {
                    self.compare_field_greater(field_bytes, value, column_type)
                } else {
                    false
                }
            }
            Some(FilterOperator::FILTER_OPERATOR_GTE) => {
                if let Some(value) = condition.value.as_ref() {
                    self.compare_field_equals(field_bytes, value, column_type) || 
                    self.compare_field_greater(field_bytes, value, column_type)
                } else {
                    false
                }
            }
            Some(FilterOperator::FILTER_OPERATOR_LT) => {
                if let Some(value) = condition.value.as_ref() {
                    self.compare_field_less(field_bytes, value, column_type)
                } else {
                    false
                }
            }
            Some(FilterOperator::FILTER_OPERATOR_LTE) => {
                if let Some(value) = condition.value.as_ref() {
                    self.compare_field_equals(field_bytes, value, column_type) || 
                    self.compare_field_less(field_bytes, value, column_type)
                } else {
                    false
                }
            }
            Some(FilterOperator::FILTER_OPERATOR_IN) => {
                if !condition.values.is_empty() {
                    for value in &condition.values {
                        if self.compare_field_equals(field_bytes, value, column_type) {
                            return true;
                        }
                    }
                }
                false
            }
            Some(FilterOperator::FILTER_OPERATOR_NOT_IN) => {
                if !condition.values.is_empty() {
                    for value in &condition.values {
                        if self.compare_field_equals(field_bytes, value, column_type) {
                            return false;
                        }
                    }
                    return true;
                }
                false
            }
            Some(FilterOperator::FILTER_OPERATOR_IS_NULL) => field_bytes.is_empty(),
            Some(FilterOperator::FILTER_OPERATOR_IS_NOT_NULL) => !field_bytes.is_empty(),
            _ => false,
        }
    }

    fn compare_field_equals(
        &self, 
        field_bytes: &[u8], 
        field: &laoflchdb_engines::Field, 
        _column_type: ColumnType
    ) -> bool {
        use laoflchdb_engines::field::field::Value;
        
        let row_field = match self.parse_field_from_bytes(field_bytes) {
            Ok(f) => f,
            Err(_) => return false,
        };
        
        if let (Some(ref row_value), Some(ref value)) = (&row_field.value, &field.value) {
            match (row_value, value) {
                (Value::StringValue(s1), Value::StringValue(s2)) => s1.value == s2.value,
                (Value::IntegerValue(i1), Value::IntegerValue(i2)) => i1.value == i2.value,
                (Value::BytesValue(b1), Value::BytesValue(b2)) => b1.value == b2.value,
                (Value::FloatValue(f1), Value::FloatValue(f2)) => f1.value == f2.value,
                _ => false,
            }
        } else {
            false
        }
    }

    fn compare_field_greater(
        &self, 
        field_bytes: &[u8], 
        field: &laoflchdb_engines::Field, 
        _column_type: ColumnType
    ) -> bool {
        use laoflchdb_engines::field::field::Value;
        
        let row_field = match self.parse_field_from_bytes(field_bytes) {
            Ok(f) => f,
            Err(_) => return false,
        };
        
        if let (Some(ref row_value), Some(ref value)) = (&row_field.value, &field.value) {
            match (row_value, value) {
                (Value::StringValue(s1), Value::StringValue(s2)) => s1.value > s2.value,
                (Value::IntegerValue(i1), Value::IntegerValue(i2)) => i1.value > i2.value,
                (Value::FloatValue(f1), Value::FloatValue(f2)) => f1.value > f2.value,
                (Value::IntegerValue(i1), Value::StringValue(s2)) => {
                    match s2.value.parse::<i64>() {
                        Ok(val) => i1.value > val,
                        Err(_) => false,
                    }
                }
                (Value::StringValue(s1), Value::IntegerValue(i2)) => {
                    match s1.value.parse::<i64>() {
                        Ok(val) => val > i2.value,
                        Err(_) => false,
                    }
                }
                (Value::FloatValue(f1), Value::StringValue(s2)) => {
                    match s2.value.parse::<f64>() {
                        Ok(val) => f1.value > val,
                        Err(_) => false,
                    }
                }
                (Value::StringValue(s1), Value::FloatValue(f2)) => {
                    match s1.value.parse::<f64>() {
                        Ok(val) => val > f2.value,
                        Err(_) => false,
                    }
                }
                _ => false,
            }
        } else {
            false
        }
    }

    fn compare_field_less(
        &self, 
        field_bytes: &[u8], 
        field: &laoflchdb_engines::Field, 
        _column_type: ColumnType
    ) -> bool {
        use laoflchdb_engines::field::field::Value;
        
        let row_field = match self.parse_field_from_bytes(field_bytes) {
            Ok(f) => f,
            Err(_) => return false,
        };
        
        if let (Some(ref row_value), Some(ref value)) = (&row_field.value, &field.value) {
            match (row_value, value) {
                (Value::StringValue(s1), Value::StringValue(s2)) => s1.value < s2.value,
                (Value::IntegerValue(i1), Value::IntegerValue(i2)) => i1.value < i2.value,
                (Value::FloatValue(f1), Value::FloatValue(f2)) => f1.value < f2.value,
                (Value::IntegerValue(i1), Value::StringValue(s2)) => {
                    match s2.value.parse::<i64>() {
                        Ok(val) => i1.value < val,
                        Err(_) => false,
                    }
                }
                (Value::StringValue(s1), Value::IntegerValue(i2)) => {
                    match s1.value.parse::<i64>() {
                        Ok(val) => val < i2.value,
                        Err(_) => false,
                    }
                }
                (Value::FloatValue(f1), Value::StringValue(s2)) => {
                    match s2.value.parse::<f64>() {
                        Ok(val) => f1.value < val,
                        Err(_) => false,
                    }
                }
                (Value::StringValue(s1), Value::FloatValue(f2)) => {
                    match s1.value.parse::<f64>() {
                        Ok(val) => val < f2.value,
                        Err(_) => false,
                    }
                }
                _ => false,
            }
        } else {
            false
        }
    }
    
    pub fn column_type_to_arrow_type(&self, col_type: &ColumnType) -> DataType {
        match col_type {
            ColumnType::COLUMN_TYPE_STRING => DataType::Utf8,
            ColumnType::COLUMN_TYPE_INT64 => DataType::Int64,
            ColumnType::COLUMN_TYPE_FLOAT => DataType::Float64,
            ColumnType::COLUMN_TYPE_BYTES => DataType::Binary,
            _ => DataType::Utf8,
        }
    }
    
    fn get_enum_value(&self, col_type: &ColumnType) -> i32 {
        *col_type as i32
    }
    
    #[inline]
    fn parse_field_from_bytes(&self, field_bytes: &[u8]) -> Result<laoflchdb_engines::Field, protobuf::Error> {
        let mut input = protobuf::CodedInputStream::from_bytes(field_bytes);
        laoflchdb_engines::Field::parse_from(&mut input)
    }
    
    pub async fn table_to_arrow_with_pushdown(
        &self, 
        table_name: &str,
        projection: Option<&Vec<usize>>,
        filters: &[ColumnFilter],
        limit: Option<usize>,
        negate_result: bool
    ) -> Result<(Schema, Vec<ArrayRef>, Vec<(i32, String)>), Box<dyn std::error::Error + Send + Sync>> {
        let columns = StorageEngine::list_table_cols(self, table_name).await?;
        
        let projected_columns: Vec<ColumnMeta> = match projection {
            Some(p) => p.iter()
                .filter(|&&idx| idx < columns.len())
                .map(|&idx| columns[idx].clone())
                .collect(),
            None => columns.clone(),
        };
        
        let projected_column_names: Vec<String> = projected_columns.iter()
            .map(|col| col.column_name.clone())
            .collect();
        
        let query = Query {
            table_filters: vec![TableFilter {
                table_name: table_name.to_string(),
                column_filters: filters.to_vec(),
                special_fields: ::protobuf::SpecialFields::default(),
            }],
            limit: limit.map(|l| l as u32),
            offset: None,
            projected_columns: projected_column_names,
            special_fields: ::protobuf::SpecialFields::default(),
        };
        
        let result = self.query(&query).await?;
        
        let mut column_infos: Vec<(i32, String)> = Vec::new();
        let mut arrow_fields = Vec::new();
        let mut arrow_arrays: Vec<Vec<ArrayRef>> = Vec::new();
        
        for col in &projected_columns {
            let col_type = col.column_type.enum_value_or_default();
            let data_type = self.column_type_to_arrow_type(&col_type);
            column_infos.push((self.get_enum_value(&col_type), col.column_name.clone()));
            arrow_fields.push(ArrowField::new(&col.column_name, data_type, true));
            arrow_arrays.push(Vec::new());
        }
        
        let column_name_to_idx: std::collections::HashMap<String, usize> = columns.iter()
            .enumerate()
            .map(|(idx, col)| (col.column_name.clone(), idx))
            .collect();
        
        for qr in &result.rows {
            if let Some(row) = qr.row.as_ref() {
                let mut row_arrays: Vec<Option<ArrayRef>> = vec![None; projected_columns.len()];
                
                for (arr_idx, col) in projected_columns.iter().enumerate() {
                    if let Some(&orig_idx) = column_name_to_idx.get(&col.column_name) {
                        if orig_idx >= row.data.len() {
                            continue;
                        }
                        
                        let field_bytes = &row.data[orig_idx];
                        
                        let pb_field = match self.parse_field_from_bytes(field_bytes) {
                            Ok(f) => f,
                            Err(_) => continue,
                        };
                        
                        use laoflchdb_engines::field::field::Value;
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
                        row_arrays[arr_idx] = Some(array);
                    }
                }
                
                for (arr_idx, arr) in row_arrays.iter().enumerate() {
                    let array = match arr {
                        Some(a) => a.clone(),
                        None => {
                            let (col_type, _) = &column_infos[arr_idx];
                            match *col_type {
                                0 => Arc::new(StringArray::from(vec![""])) as ArrayRef,
                                1 => Arc::new(Int64Array::from(vec![0])) as ArrayRef,
                                3 => Arc::new(Float64Array::from(vec![0.0])) as ArrayRef,
                                2 => Arc::new(BinaryArray::from(vec![&[][..]])) as ArrayRef,
                                _ => Arc::new(StringArray::from(vec![""])) as ArrayRef,
                            }
                        }
                    };
                    arrow_arrays[arr_idx].push(array);
                }
            }
        }
        
        let total_rows = arrow_arrays.iter().map(|a| a.len()).max().unwrap_or(0);
        
        let merged_arrays: Vec<ArrayRef> = arrow_arrays.into_iter()
            .enumerate()
            .map(|(idx, arrays)| {
                if arrays.is_empty() {
                    let (col_type, _) = &column_infos[idx];
                    match *col_type {
                        0 => Arc::new(StringArray::from(vec![""; total_rows])) as ArrayRef,
                        1 => Arc::new(Int64Array::from(vec![0i64; total_rows])) as ArrayRef,
                        3 => Arc::new(Float64Array::from(vec![0.0f64; total_rows])) as ArrayRef,
                        2 => Arc::new(BinaryArray::from(vec![&[][..]; total_rows])) as ArrayRef,
                        _ => Arc::new(StringArray::from(vec![""; total_rows])) as ArrayRef,
                    }
                } else if arrays.len() == 1 {
                    arrays[0].clone()
                } else {
                    let refs: Vec<&dyn datafusion::arrow::array::Array> = arrays.iter().map(|a| a.as_ref()).collect();
                    match datafusion::arrow::compute::concat(refs.as_slice()) {
                        Ok(arr) => arr,
                        Err(e) => {
                            log::warn!("Failed to concatenate arrays for column {}: {}", idx, e);
                            let (col_type, _) = &column_infos[idx];
                            match *col_type {
                                0 => Arc::new(StringArray::from(vec![""; total_rows])) as ArrayRef,
                                1 => Arc::new(Int64Array::from(vec![0i64; total_rows])) as ArrayRef,
                                3 => Arc::new(Float64Array::from(vec![0.0f64; total_rows])) as ArrayRef,
                                2 => Arc::new(BinaryArray::from(vec![&[][..]; total_rows])) as ArrayRef,
                                _ => Arc::new(StringArray::from(vec![""; total_rows])) as ArrayRef,
                            }
                        }
                    }
                }
            })
            .collect();
        
        let schema = Schema::new(arrow_fields);
        Ok((schema, merged_arrays, column_infos))
    }
    
    /// 使用 FilterGroup 进行过滤
    pub async fn table_to_arrow_with_filter_group(
        &self, 
        table_name: &str,
        projection: Option<&Vec<usize>>,
        filter_group: &FilterGroup,
        limit: Option<usize>,
        negate_result: bool
    ) -> Result<(Schema, Vec<ArrayRef>, Vec<(i32, String)>), Box<dyn std::error::Error + Send + Sync>> {
        let columns = StorageEngine::list_table_cols(self, table_name).await?;
        let column_types = self.get_column_types(table_name).await?;
        
        let projected_columns: Vec<ColumnMeta> = match projection {
            Some(p) => p.iter()
                .filter(|&&idx| idx < columns.len())
                .map(|&idx| columns[idx].clone())
                .collect(),
            None => columns.clone(),
        };
        
        // 使用 FilterGroup 进行过滤
        let rows = self.query_table_with_filter_group(
            table_name, 
            filter_group, 
            negate_result
        ).await?;
        
        let mut column_infos: Vec<(i32, String)> = Vec::new();
        let mut arrow_fields = Vec::new();
        let mut arrow_arrays: Vec<Vec<ArrayRef>> = Vec::new();
        
        for col in &projected_columns {
            let col_type = col.column_type.enum_value_or_default();
            let data_type = self.column_type_to_arrow_type(&col_type);
            column_infos.push((self.get_enum_value(&col_type), col.column_name.clone()));
            arrow_fields.push(ArrowField::new(&col.column_name, data_type, true));
            arrow_arrays.push(Vec::new());
        }
        
        let column_name_to_idx: std::collections::HashMap<String, usize> = columns.iter()
            .enumerate()
            .map(|(idx, col)| (col.column_name.clone(), idx))
            .collect();
        
        // 应用 limit
        let limited_rows: Vec<_> = if let Some(l) = limit {
            rows.into_iter().take(l).collect()
        } else {
            rows
        };
        
        for (row_id, row) in limited_rows {
            let mut row_arrays: Vec<Option<ArrayRef>> = vec![None; projected_columns.len()];
            
            for (arr_idx, col) in projected_columns.iter().enumerate() {
                if let Some(&orig_idx) = column_name_to_idx.get(&col.column_name) {
                    if orig_idx >= row.data.len() {
                        continue;
                    }
                    
                    let field_bytes = &row.data[orig_idx];
                    
                    let pb_field = match self.parse_field_from_bytes(field_bytes) {
                        Ok(f) => f,
                        Err(_) => continue,
                    };
                    
                    use laoflchdb_engines::field::field::Value;
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
                    row_arrays[arr_idx] = Some(array);
                }
            }
            
            for (arr_idx, arr) in row_arrays.iter().enumerate() {
                let array = match arr {
                    Some(a) => a.clone(),
                    None => {
                        let (col_type, _) = &column_infos[arr_idx];
                        match *col_type {
                            0 => Arc::new(StringArray::from(vec![""])) as ArrayRef,
                            1 => Arc::new(Int64Array::from(vec![0])) as ArrayRef,
                            3 => Arc::new(Float64Array::from(vec![0.0])) as ArrayRef,
                            2 => Arc::new(BinaryArray::from(vec![&[][..]])) as ArrayRef,
                            _ => Arc::new(StringArray::from(vec![""])) as ArrayRef,
                        }
                    }
                };
                arrow_arrays[arr_idx].push(array);
            }
        }
        
        let total_rows = arrow_arrays.iter().map(|a| a.len()).max().unwrap_or(0);
        
        let merged_arrays: Vec<ArrayRef> = arrow_arrays.into_iter()
            .enumerate()
            .map(|(idx, arrays)| {
                if arrays.is_empty() {
                    let (col_type, _) = &column_infos[idx];
                    match *col_type {
                        0 => Arc::new(StringArray::from(vec![""; total_rows])) as ArrayRef,
                        1 => Arc::new(Int64Array::from(vec![0i64; total_rows])) as ArrayRef,
                        3 => Arc::new(Float64Array::from(vec![0.0f64; total_rows])) as ArrayRef,
                        2 => Arc::new(BinaryArray::from(vec![&[][..]; total_rows])) as ArrayRef,
                        _ => Arc::new(StringArray::from(vec![""; total_rows])) as ArrayRef,
                    }
                } else if arrays.len() == 1 {
                    arrays[0].clone()
                } else {
                    let refs: Vec<&dyn datafusion::arrow::array::Array> = arrays.iter().map(|a| a.as_ref()).collect();
                    match datafusion::arrow::compute::concat(refs.as_slice()) {
                        Ok(arr) => arr,
                        Err(e) => {
                            log::warn!("Failed to concatenate arrays for column {}: {}", idx, e);
                            let (col_type, _) = &column_infos[idx];
                            match *col_type {
                                0 => Arc::new(StringArray::from(vec![""; total_rows])) as ArrayRef,
                                1 => Arc::new(Int64Array::from(vec![0i64; total_rows])) as ArrayRef,
                                3 => Arc::new(Float64Array::from(vec![0.0f64; total_rows])) as ArrayRef,
                                2 => Arc::new(BinaryArray::from(vec![&[][..]; total_rows])) as ArrayRef,
                                _ => Arc::new(StringArray::from(vec![""; total_rows])) as ArrayRef,
                            }
                        }
                    }
                }
            })
            .collect();
        
        let schema = Schema::new(arrow_fields);
        Ok((schema, merged_arrays, column_infos))
    }
    
    /// 同步版本：列出表的列
    fn list_table_cols_sync(&self, table: &str) -> Result<Vec<ColumnMeta>, Box<dyn std::error::Error + Send + Sync>> {
        let mut cols = Vec::new();
        
        let table_prefix = format!("{}:{}:", META_TABLE_PREFIX, table);
        let table_entries = self.scan_meta_prefix_internal(table_prefix.as_bytes())?;
        
        let mut table_id = None;
        for (_, value) in table_entries {
            if let Ok(table_meta) = TableMeta::parse_from_bytes(value.as_slice()) {
                table_id = Some(table_meta.table_id);
                break;
            }
        }
        
        if let Some(tid) = table_id {
            let col_prefix = format!("{}:{}:", META_COLUMN_PREFIX, self.format_table_id(tid));
            let col_entries = self.scan_meta_prefix_internal(col_prefix.as_bytes())?;
            
            for (_, value) in col_entries {
                if let Ok(col_meta) = ColumnMeta::parse_from_bytes(value.as_slice()) {
                    cols.push(col_meta);
                }
            }
        }
        
        cols.sort_by_key(|c| c.column_id);
        Ok(cols)
    }
    
    /// 同步版本：获取列类型
    fn get_column_types_sync(&self, table_name: &str) -> Result<std::collections::HashMap<String, ColumnType>, Box<dyn std::error::Error + Send + Sync>> {
        let mut column_types = std::collections::HashMap::new();
        
        let cols = self.list_table_cols_sync(table_name)?;
        for col in cols {
            column_types.insert(col.column_name.clone(), col.column_type.enum_value_or_default());
        }
        
        Ok(column_types)
    }
    
    /// 同步版本：使用 FilterGroup 查询表
    fn query_table_with_filter_group_sync(
        &self, 
        table_name: &str, 
        filter_group: &FilterGroup,
        negate_result: bool
    ) -> Result<Vec<(u64, Row)>, Box<dyn std::error::Error + Send + Sync>> {
        let cf_name = self.get_table_cf(table_name);
        let db = self.db.read().unwrap();
        let cf_handle = match db.cf_handle(&cf_name) {
            Some(handle) => handle,
            None => return Ok(Vec::new()),
        };
        
        let columns = self.get_table_columns(table_name)?;
        
        let mut result_rows = Vec::new();
        
        let iter = db.iterator_cf(cf_handle, rocksdb::IteratorMode::Start);
        for item in iter {
            let (key, value) = item?;
            
            let row_id = match self.key_to_row_id(key.as_ref()) {
                Ok(id) => id,
                Err(_) => continue,
            };
            
            let row = match Row::parse_from_bytes(&value[..]) {
                Ok(row) => row,
                Err(_) => continue,
            };
            
            // 使用 FilterGroup 检查过滤条件
            let matches = self.check_filter_group(&row, filter_group, &columns);
            
            if negate_result {
                // 取反：返回不满足条件的行
                if !matches {
                    result_rows.push((row_id, row));
                }
            } else {
                // 正常：返回满足条件的行
                if matches {
                    result_rows.push((row_id, row));
                }
            }
        }
        
        Ok(result_rows)
    }
    
    /// 同步版本：使用 FilterGroup 进行过滤
    pub fn table_to_arrow_with_filter_group_sync(
        &self, 
        table_name: &str,
        projection: Option<&Vec<usize>>,
        filter_group: &FilterGroup,
        limit: Option<usize>,
        negate_result: bool
    ) -> Result<(Schema, Vec<ArrayRef>, Vec<(i32, String)>), Box<dyn std::error::Error + Send + Sync>> {
        let columns = self.list_table_cols_sync(table_name)?;
        let column_types = self.get_column_types_sync(table_name)?;
        
        let projected_columns: Vec<ColumnMeta> = match projection {
            Some(p) => p.iter()
                .filter(|&&idx| idx < columns.len())
                .map(|&idx| columns[idx].clone())
                .collect(),
            None => columns.clone(),
        };
        
        // 使用 FilterGroup 进行过滤
        let rows = self.query_table_with_filter_group_sync(
            table_name, 
            filter_group, 
            negate_result
        )?;
        
        let mut column_infos: Vec<(i32, String)> = Vec::new();
        let mut arrow_fields = Vec::new();
        let mut arrow_arrays: Vec<Vec<ArrayRef>> = Vec::new();
        
        for col in &projected_columns {
            let col_type = col.column_type.enum_value_or_default();
            let data_type = self.column_type_to_arrow_type(&col_type);
            column_infos.push((self.get_enum_value(&col_type), col.column_name.clone()));
            arrow_fields.push(ArrowField::new(&col.column_name, data_type, true));
            arrow_arrays.push(Vec::new());
        }
        
        let column_name_to_idx: std::collections::HashMap<String, usize> = columns.iter()
            .enumerate()
            .map(|(idx, col)| (col.column_name.clone(), idx))
            .collect();
        
        // 应用 limit
        let limited_rows: Vec<_> = if let Some(l) = limit {
            rows.into_iter().take(l).collect()
        } else {
            rows
        };
        
        for (row_id, row) in limited_rows {
            let mut row_arrays: Vec<Option<ArrayRef>> = vec![None; projected_columns.len()];
            
            for (arr_idx, col) in projected_columns.iter().enumerate() {
                if let Some(&orig_idx) = column_name_to_idx.get(&col.column_name) {
                    if orig_idx >= row.data.len() {
                        continue;
                    }
                    
                    let field_bytes = &row.data[orig_idx];
                    
                    let pb_field = match self.parse_field_from_bytes(field_bytes) {
                        Ok(f) => f,
                        Err(_) => continue,
                    };
                    
                    use laoflchdb_engines::field::field::Value;
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
                    row_arrays[arr_idx] = Some(array);
                }
            }
            
            for (arr_idx, arr) in row_arrays.iter().enumerate() {
                let array = match arr {
                    Some(a) => a.clone(),
                    None => {
                        let (col_type, _) = &column_infos[arr_idx];
                        match *col_type {
                            0 => Arc::new(StringArray::from(vec![""])) as ArrayRef,
                            1 => Arc::new(Int64Array::from(vec![0])) as ArrayRef,
                            3 => Arc::new(Float64Array::from(vec![0.0])) as ArrayRef,
                            2 => Arc::new(BinaryArray::from(vec![&[][..]])) as ArrayRef,
                            _ => Arc::new(StringArray::from(vec![""])) as ArrayRef,
                        }
                    }
                };
                arrow_arrays[arr_idx].push(array);
            }
        }
        
        let total_rows = arrow_arrays.iter().map(|a| a.len()).max().unwrap_or(0);
        
        let merged_arrays: Vec<ArrayRef> = arrow_arrays.into_iter()
            .enumerate()
            .map(|(idx, arrays)| {
                if arrays.is_empty() {
                    let (col_type, _) = &column_infos[idx];
                    match *col_type {
                        0 => Arc::new(StringArray::from(vec![""; total_rows])) as ArrayRef,
                        1 => Arc::new(Int64Array::from(vec![0i64; total_rows])) as ArrayRef,
                        3 => Arc::new(Float64Array::from(vec![0.0f64; total_rows])) as ArrayRef,
                        2 => Arc::new(BinaryArray::from(vec![&[][..]; total_rows])) as ArrayRef,
                        _ => Arc::new(StringArray::from(vec![""; total_rows])) as ArrayRef,
                    }
                } else if arrays.len() == 1 {
                    arrays[0].clone()
                } else {
                    let refs: Vec<&dyn datafusion::arrow::array::Array> = arrays.iter().map(|a| a.as_ref()).collect();
                    match datafusion::arrow::compute::concat(refs.as_slice()) {
                        Ok(arr) => arr,
                        Err(e) => {
                            log::warn!("Failed to concatenate arrays for column {}: {}", idx, e);
                            let (col_type, _) = &column_infos[idx];
                            match *col_type {
                                0 => Arc::new(StringArray::from(vec![""; total_rows])) as ArrayRef,
                                1 => Arc::new(Int64Array::from(vec![0i64; total_rows])) as ArrayRef,
                                3 => Arc::new(Float64Array::from(vec![0.0f64; total_rows])) as ArrayRef,
                                2 => Arc::new(BinaryArray::from(vec![&[][..]; total_rows])) as ArrayRef,
                                _ => Arc::new(StringArray::from(vec![""; total_rows])) as ArrayRef,
                            }
                        }
                    }
                }
            })
            .collect();
        
        let schema = Schema::new(arrow_fields);
        Ok((schema, merged_arrays, column_infos))
    }
    
    /// 使用 FilterGroup 查询表
    async fn query_table_with_filter_group(
        &self, 
        table_name: &str, 
        filter_group: &FilterGroup,
        negate_result: bool
    ) -> Result<Vec<(u64, Row)>, Box<dyn std::error::Error + Send + Sync>> {
        let cf_name = self.get_table_cf(table_name);
        let db = self.db.read().unwrap();
        let cf_handle = match db.cf_handle(&cf_name) {
            Some(handle) => handle,
            None => return Ok(Vec::new()),
        };
        
        let columns = self.get_table_columns(table_name)?;
        
        let mut result_rows = Vec::new();
        
        let iter = db.iterator_cf(cf_handle, rocksdb::IteratorMode::Start);
        for item in iter {
            let (key, value) = item?;
            
            let row_id = match self.key_to_row_id(key.as_ref()) {
                Ok(id) => id,
                Err(_) => continue,
            };
            
            let row = match Row::parse_from_bytes(&value[..]) {
                Ok(row) => row,
                Err(_) => continue,
            };
            
            // 使用 FilterGroup 检查过滤条件
            let matches = self.check_filter_group(&row, filter_group, &columns);
            
            if negate_result {
                // 取反：返回不满足条件的行
                if !matches {
                    result_rows.push((row_id, row));
                }
            } else {
                // 正常：返回满足条件的行
                if matches {
                    result_rows.push((row_id, row));
                }
            }
        }
        
        Ok(result_rows)
    }
    
    /// 检查行是否满足 FilterGroup 的条件
    fn check_filter_group(
        &self, 
        row: &Row, 
        filter_group: &FilterGroup,
        columns: &std::collections::HashMap<String, (u64, ColumnType)>
    ) -> bool {
        if filter_group.items.is_empty() {
            return true; // 空过滤器组，返回 true
        }
        
        match filter_group.relation {
            FilterRelation::And => {
                // AND 关系：所有项都要满足
                for item in &filter_group.items {
                    if !self.check_filter_item(row, item, columns) {
                        return false;
                    }
                }
                true
            }
            FilterRelation::Or => {
                // OR 关系：任一项满足即可
                for item in &filter_group.items {
                    if self.check_filter_item(row, item, columns) {
                        return true;
                    }
                }
                false
            }
        }
    }
    
    /// 检查行是否满足 FilterItem 的条件
    fn check_filter_item(
        &self, 
        row: &Row, 
        filter_item: &FilterItem,
        columns: &std::collections::HashMap<String, (u64, ColumnType)>
    ) -> bool {
        match filter_item {
            FilterItem::ColumnFilter(filter) => {
                self.check_column_filter(row, filter, columns)
            }
            FilterItem::Group(group) => {
                self.check_filter_group(row, group, columns)
            }
        }
    }
    
    pub async fn table_to_arrow(&self, table_name: &str) -> Result<(Schema, Vec<ArrayRef>, Vec<(i32, String)>), Box<dyn std::error::Error + Send + Sync>> {
        let columns = StorageEngine::list_table_cols(self, table_name).await?;
        let rows = StorageEngine::scan_table(self, table_name, None).await?;
        
        let mut column_infos: Vec<(i32, String)> = Vec::new();
        let mut arrow_fields = Vec::new();
        let mut arrow_arrays: Vec<Vec<ArrayRef>> = Vec::new();
        
        for col in &columns {
            let col_type = col.column_type.enum_value_or_default();
            let data_type = self.column_type_to_arrow_type(&col_type);
            column_infos.push((self.get_enum_value(&col_type), col.column_name.clone()));
            arrow_fields.push(ArrowField::new(&col.column_name, data_type, true));
            arrow_arrays.push(Vec::new());
        }
        
        for (_, row) in rows {
            for (idx, field_bytes) in row.data.iter().enumerate() {
                if idx >= arrow_arrays.len() {
                    break;
                }
                
                let pb_field = match self.parse_field_from_bytes(field_bytes) {
                    Ok(f) => f,
                    Err(_) => continue,
                };
                
                use laoflchdb_engines::field::field::Value;
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
        
        let total_rows = arrow_arrays.iter().map(|a| a.len()).max().unwrap_or(0);
        
        let merged_arrays: Vec<ArrayRef> = arrow_arrays.into_iter()
            .enumerate()
            .map(|(idx, arrays)| {
                if arrays.is_empty() {
                    let (col_type, _) = &column_infos[idx];
                    match *col_type {
                        0 => Arc::new(StringArray::from(vec![""; total_rows])) as ArrayRef,
                        1 => Arc::new(Int64Array::from(vec![0i64; total_rows])) as ArrayRef,
                        3 => Arc::new(Float64Array::from(vec![0.0f64; total_rows])) as ArrayRef,
                        2 => Arc::new(BinaryArray::from(vec![&[][..]; total_rows])) as ArrayRef,
                        _ => Arc::new(StringArray::from(vec![""; total_rows])) as ArrayRef,
                    }
                } else if arrays.len() == 1 {
                    arrays[0].clone()
                } else {
                    let refs: Vec<&dyn datafusion::arrow::array::Array> = arrays.iter().map(|a| a.as_ref()).collect();
                    match datafusion::arrow::compute::concat(refs.as_slice()) {
                        Ok(arr) => arr,
                        Err(e) => {
                            log::warn!("Failed to concatenate arrays for column {}: {}", idx, e);
                            let (col_type, _) = &column_infos[idx];
                            match *col_type {
                                0 => Arc::new(StringArray::from(vec![""; total_rows])) as ArrayRef,
                                1 => Arc::new(Int64Array::from(vec![0i64; total_rows])) as ArrayRef,
                                3 => Arc::new(Float64Array::from(vec![0.0f64; total_rows])) as ArrayRef,
                                2 => Arc::new(BinaryArray::from(vec![&[][..]; total_rows])) as ArrayRef,
                                _ => Arc::new(StringArray::from(vec![""; total_rows])) as ArrayRef,
                            }
                        }
                    }
                }
            })
            .collect();
        
        let schema = Schema::new(arrow_fields);
        Ok((schema, merged_arrays, column_infos))
    }
    
    fn create_table_provider(&self, table_name: &str) -> Arc<dyn TableProvider> {
        use std::thread;
        use std::sync::mpsc;
        
        let engine = Arc::new(self.clone());
        let table_name_clone = table_name.to_string();
        
        let (tx, rx) = mpsc::channel();
        
        thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            let table = rt.block_on(async move {
                RocksDBTable::new(engine, &table_name_clone).await
            });
            let _ = tx.send(table);
        });
        
        Arc::new(rx.recv().unwrap())
    }
}

#[async_trait::async_trait]
impl DataFusionStorageEngine for MultiTableRocksDBEngine {
    async fn table_to_arrow(&self, table_name: &str) -> Result<(Schema, Vec<ArrayRef>, Vec<(i32, String)>), Box<dyn std::error::Error + Send + Sync>> {
        self.table_to_arrow(table_name).await
    }
    
    fn create_table_provider(&self, table_name: &str) -> Arc<dyn TableProvider> {
        use std::thread;
        use std::sync::mpsc;
        
        let engine = Arc::new(self.clone());
        let table_name_clone = table_name.to_string();
        
        let (tx, rx) = mpsc::channel();
        
        thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            let table = rt.block_on(async move {
                RocksDBTable::new(engine, &table_name_clone).await
            });
            let _ = tx.send(table);
        });
        
        Arc::new(rx.recv().unwrap())
    }
}


