use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use log::info;
use rocksdb::{DB, Options, ColumnFamilyDescriptor, IteratorMode};
use thiserror::Error;

use laoflchdb_engines::{
    ColumnMeta, ColumnType, EngineOptions, Query, QueryResult, Row,
    SchemaMeta, SpecialFields, StorageEngine, TableMeta,
    META_SCHEMA_PREFIX, META_TABLE_PREFIX, META_COLUMN_PREFIX,
};

fn write_proto_to_vec<T: protobuf::Message>(msg: &T) -> Vec<u8> {
    let mut v = Vec::new();
    msg.write_to_vec(&mut v).expect("Failed to serialize protobuf");
    v
}

#[derive(Error, Debug)]
pub enum KVRocksDBError {
    #[error("RocksDB error: {0}")]
    RocksDB(#[from] rocksdb::Error),
    #[error("Engine error: {0}")]
    Engine(String),
    #[error("Protobuf error: {0}")]
    Protobuf(#[from] protobuf::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Clone)]
pub struct KVRocksDBEngine {
    db: Arc<RwLock<DB>>,
    schema_name: String,
}

impl std::fmt::Debug for KVRocksDBEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KVRocksDBEngine")
            .field("schema_name", &self.schema_name)
            .finish()
    }
}

impl KVRocksDBEngine {
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
            cf_list
                .into_iter()
                .map(|name| ColumnFamilyDescriptor::new(name, Options::default()))
                .collect()
        };

        let db = DB::open_cf_descriptors(&opts, path, cf_descriptors)?;
        let db = Arc::new(RwLock::new(db));

        let engine = Self { db, schema_name };
        engine.init_schema_meta()?;

        Ok(engine)
    }

    fn init_schema_meta(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let key = self.make_schema_meta_key();
        if self.get_meta(key.as_bytes())?.is_none() {
            let schema_meta = SchemaMeta {
                schema_name: self.schema_name.clone(),
                next_auto_inc_table_id: 0,
                special_fields: SpecialFields::default(),
            };
            let encoded = write_proto_to_vec(&schema_meta);
            self.put_meta(key.as_bytes(), &encoded)?;
        }
        Ok(())
    }

    fn make_schema_meta_key(&self) -> String {
        format!("{}:{}", META_SCHEMA_PREFIX, self.schema_name)
    }

    fn make_table_meta_key(&self, table_name: &str, table_id: u64) -> String {
        format!("{}:{}:{}", META_TABLE_PREFIX, table_name, table_id)
    }

    fn make_column_meta_key(&self, table_id: u64, column_name: &str, column_id: u64) -> String {
        format!(
            "{}:{:020}:{}:{}",
            META_COLUMN_PREFIX, table_id, column_name, column_id
        )
    }

    fn get_cf_name(table: &str) -> String {
        table.to_string()
    }

    fn put_meta(&self, key: &[u8], value: &[u8]) -> Result<(), KVRocksDBError> {
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle("default")
            .ok_or_else(|| KVRocksDBError::Engine("Default column family not found".into()))?;
        db.put_cf(cf, key, value)?;
        Ok(())
    }

    fn get_meta(&self, key: &[u8]) -> Result<Option<Vec<u8>>, KVRocksDBError> {
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle("default")
            .ok_or_else(|| KVRocksDBError::Engine("Default column family not found".into()))?;
        Ok(db.get_cf(cf, key)?)
    }

    fn scan_meta_prefix(&self, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>, KVRocksDBError> {
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle("default")
            .ok_or_else(|| KVRocksDBError::Engine("Default column family not found".into()))?;
        let mut result = Vec::new();
        let mut iter = db.raw_iterator_cf(cf);
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

    fn delete_meta(&self, key: &[u8]) -> Result<(), KVRocksDBError> {
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle("default")
            .ok_or_else(|| KVRocksDBError::Engine("Default column family not found".into()))?;
        db.delete_cf(cf, key)?;
        Ok(())
    }

    fn get_next_table_id(&self) -> Result<u64, KVRocksDBError> {
        let key = self.make_schema_meta_key();
        let data = self.get_meta(key.as_bytes())?;
        if let Some(bytes) = data {
            let mut meta: SchemaMeta = protobuf::Message::parse_from_bytes(&bytes)?;
            let next = meta.next_auto_inc_table_id;
            meta.next_auto_inc_table_id = next + 1;
            self.put_meta(key.as_bytes(), &write_proto_to_vec(&meta))?;
            Ok(next)
        } else {
            Ok(0)
        }
    }

    fn key_to_bytes(key: &[u8]) -> Vec<u8> {
        key.to_vec()
    }

    pub fn db_path(&self) -> String {
        self.db.read().unwrap().path().to_string_lossy().to_string()
    }

    pub fn list_keys(&self, table: &str, prefix: Option<&[u8]>, limit: Option<usize>) -> Result<Vec<Vec<u8>>, KVRocksDBError> {
        let cf_name = Self::get_cf_name(table);
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&cf_name)
            .ok_or_else(|| KVRocksDBError::Engine(format!("Table '{}' not found", cf_name)))?;

        let mut keys = Vec::new();
        let mut iter = match prefix {
            Some(p) => {
                let mut it = db.raw_iterator_cf(cf);
                it.seek(p);
                it
            }
            None => {
                let mut it = db.raw_iterator_cf(cf);
                it.seek_to_first();
                it
            }
        };

        while iter.valid() {
            if let Some(k) = iter.key() {
                let take = match prefix {
                    Some(p) => k.starts_with(p),
                    None => true,
                };
                if take {
                    keys.push(k.to_vec());
                    if let Some(lim) = limit {
                        if keys.len() >= lim {
                            break;
                        }
                    }
                } else if prefix.is_some() {
                    break;
                }
                iter.next();
            } else {
                break;
            }
        }

        Ok(keys)
    }

    pub fn scan_range(&self, table: &str, start: &[u8], end: &[u8], limit: Option<usize>) -> Result<Vec<(Vec<u8>, Vec<u8>)>, KVRocksDBError> {
        let cf_name = Self::get_cf_name(table);
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&cf_name)
            .ok_or_else(|| KVRocksDBError::Engine(format!("Table '{}' not found", cf_name)))?;

        let mut results = Vec::new();
        let mut iter = db.raw_iterator_cf(cf);
        iter.seek(start);

        while iter.valid() {
            if let (Some(k), Some(v)) = (iter.key(), iter.value()) {
                if k >= end {
                    break;
                }
                results.push((k.to_vec(), v.to_vec()));
                if let Some(lim) = limit {
                    if results.len() >= lim {
                        break;
                    }
                }
                iter.next();
            } else {
                break;
            }
        }

        Ok(results)
    }

    pub fn batch_put(&mut self, table: &str, entries: &[(&[u8], &[u8])]) -> Result<(), KVRocksDBError> {
        let cf_name = Self::get_cf_name(table);
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&cf_name)
            .ok_or_else(|| KVRocksDBError::Engine(format!("Table '{}' not found", cf_name)))?;

        for (k, v) in entries {
            db.put_cf(cf, *k, *v)?;
        }
        Ok(())
    }

    pub fn batch_delete(&mut self, table: &str, keys: &[&[u8]]) -> Result<(), KVRocksDBError> {
        let cf_name = Self::get_cf_name(table);
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&cf_name)
            .ok_or_else(|| KVRocksDBError::Engine(format!("Table '{}' not found", cf_name)))?;

        for k in keys {
            db.delete_cf(cf, *k)?;
        }
        Ok(())
    }

    pub fn table_exists(&self, table: &str) -> bool {
        let cf_name = Self::get_cf_name(table);
        self.db.read().unwrap().cf_handle(&cf_name).is_some()
    }

    pub fn count_keys(&self, table: &str) -> Result<usize, KVRocksDBError> {
        let cf_name = Self::get_cf_name(table);
        let db = self.db.read().unwrap();
        let cf = match db.cf_handle(&cf_name) {
            Some(h) => h,
            None => return Ok(0),
        };
        let mut count = 0;
        for item in db.iterator_cf(cf, IteratorMode::Start) {
            let _ = item?;
            count += 1;
        }
        Ok(count)
    }
}

#[async_trait]
impl StorageEngine for KVRocksDBEngine {
    async fn create_table(
        &mut self,
        table: &str,
        table_comment: Option<&str>,
        columns: &[(u32, &str, ColumnType, Option<&str>)],
    ) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        if table == "default" {
            return Err("Cannot create reserved table 'default'".into());
        }

        let cf_name = Self::get_cf_name(table);
        {
            let db = self.db.read().unwrap();
            if db.cf_handle(&cf_name).is_some() {
                return Err(format!("Table '{}' already exists", table).into());
            }
        }

        let table_id = self.get_next_table_id()?;
        self.db
            .write()
            .unwrap()
            .create_cf(&cf_name, &Options::default())?;

        let table_meta = TableMeta {
            table_id,
            table_name: table.to_string(),
            column_count: columns.len() as u32,
            next_auto_inc_column_id: columns.len() as u64,
            comment: table_comment.unwrap_or("").to_string(),
            special_fields: SpecialFields::default(),
        };
        self.put_meta(
            self.make_table_meta_key(table, table_id).as_bytes(),
            &write_proto_to_vec(&table_meta),
        )?;

        for (idx, (_, col_name, col_type, col_comment)) in columns.iter().enumerate() {
            let col_meta = ColumnMeta {
                table_id,
                column_id: idx as u64,
                column_name: col_name.to_string(),
                column_type: (*col_type).into(),
                comment: col_comment.unwrap_or("").to_string(),
                special_fields: SpecialFields::default(),
            };
            self.put_meta(
                self.make_column_meta_key(table_id, col_name, idx as u64).as_bytes(),
                &write_proto_to_vec(&col_meta),
            )?;
        }

        info!("KV engine created table '{}' (id={}) with {} columns", table, table_id, columns.len());
        Ok(table_id)
    }

    async fn drop_table(&mut self, table: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let cf_name = Self::get_cf_name(table);
        {
            let db = self.db.read().unwrap();
            if db.cf_handle(&cf_name).is_some() {
                drop(db);
                self.db.write().unwrap().drop_cf(&cf_name)?;
            }
        }

        let prefix = format!("{}:{}:", META_TABLE_PREFIX, table);
        let entries = self.scan_meta_prefix(prefix.as_bytes())?;
        for (key, _) in entries {
            self.delete_meta(&key)?;
        }

        let table_meta = self.get_table_meta(table).await?;
        if let Some(tm) = table_meta {
            let col_prefix = format!("{}:{:020}:", META_COLUMN_PREFIX, tm.table_id);
            let col_entries = self.scan_meta_prefix(col_prefix.as_bytes())?;
            for (key, _) in col_entries {
                self.delete_meta(&key)?;
            }
        }

        info!("KV engine dropped table '{}'", table);
        Ok(())
    }

    async fn list_tables(&self) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        let prefix = format!("{}:", META_TABLE_PREFIX);
        let entries = self.scan_meta_prefix(prefix.as_bytes())?;
        let mut tables = std::collections::HashSet::new();
        for (key, _) in entries {
            if let Ok(k) = String::from_utf8(key) {
                if let Some(rest) = k.strip_prefix(&prefix) {
                    if let Some(name) = rest.split(':').next() {
                        tables.insert(name.to_string());
                    }
                }
            }
        }
        Ok(tables.into_iter().collect())
    }

    async fn list_table_cols(
        &self,
        table: &str,
    ) -> Result<Vec<ColumnMeta>, Box<dyn std::error::Error + Send + Sync>> {
        let mut cols = Vec::new();
        let table_meta = self.get_table_meta(table).await?;
        if let Some(tm) = table_meta {
            let col_prefix = format!("{}:{:020}:", META_COLUMN_PREFIX, tm.table_id);
            let entries = self.scan_meta_prefix(col_prefix.as_bytes())?;
            for (_, value) in entries {
                if let Ok(cm) = <ColumnMeta as protobuf::Message>::parse_from_bytes(&value) {
                    cols.push(cm);
                }
            }
        }
        cols.sort_by_key(|c: &ColumnMeta| c.column_id);
        Ok(cols)
    }

    async fn add_row(
        &mut self,
        _table: &str,
        _row: &Row,
    ) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        Err("add_row is not supported in KV engine; use put() instead".into())
    }

    async fn get_row(
        &self,
        _table: &str,
        _row_id: u64,
    ) -> Result<Option<Row>, Box<dyn std::error::Error + Send + Sync>> {
        Err("get_row is not supported in KV engine; use get() instead".into())
    }

    async fn delete_row(
        &mut self,
        _table: &str,
        _row_id: u64,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Err("delete_row is not supported in KV engine; use delete() instead".into())
    }

    async fn update_row(
        &mut self,
        _table: &str,
        _row_id: u64,
        _row: &Row,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Err("update_row is not supported in KV engine; use put() instead".into())
    }

    async fn get_all_meta(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let mut result = serde_json::Map::new();
        let mut tables = serde_json::Map::new();
        let entries = self.scan_meta_prefix(META_TABLE_PREFIX.as_bytes())?;
        for (_, value) in entries {
            if let Ok(tm) = <TableMeta as protobuf::Message>::parse_from_bytes(&value) {
                let mut obj = serde_json::Map::new();
                obj.insert("table_id".into(), serde_json::Value::from(tm.table_id));
                obj.insert("table_name".into(), serde_json::Value::from(tm.table_name.clone()));
                obj.insert("column_count".into(), serde_json::Value::from(tm.column_count));
                obj.insert("comment".into(), serde_json::Value::from(tm.comment.clone()));
                tables.insert(tm.table_name.clone(), serde_json::Value::Object(obj));
            }
        }
        result.insert("tables".into(), tables.into());
        Ok(serde_json::to_string(&result)?)
    }

    async fn get_schema_info(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let tables = self.list_tables().await?;
        let mut result = serde_json::Map::new();
        result.insert("schema_name".into(), self.schema_name.clone().into());
        result.insert("table_count".into(), tables.len().into());
        result.insert("tables".into(), tables.into());
        Ok(serde_json::to_string(&result)?)
    }

    async fn get_table_meta(
        &self,
        table: &str,
    ) -> Result<Option<TableMeta>, Box<dyn std::error::Error + Send + Sync>> {
        let prefix = format!("{}:{}:", META_TABLE_PREFIX, table);
        let entries = self.scan_meta_prefix(prefix.as_bytes())?;
        for (_, value) in entries {
            if let Ok(tm) = <TableMeta as protobuf::Message>::parse_from_bytes(&value) {
                return Ok(Some(tm));
            }
        }
        Ok(None)
    }

    async fn update_table_comment(
        &mut self,
        table: &str,
        comment: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let prefix = format!("{}:{}:", META_TABLE_PREFIX, table);
        let entries = self.scan_meta_prefix(prefix.as_bytes())?;
        for (key, value) in entries {
            let mut tm: TableMeta = <TableMeta as protobuf::Message>::parse_from_bytes(&value)?;
            tm.comment = comment.to_string();
            let db = self.db.write().unwrap();
            db.put(key, write_proto_to_vec(&tm))?;
            return Ok(());
        }
        Err(format!("Table '{}' not found", table).into())
    }

    async fn update_column_comment(
        &mut self,
        table: &str,
        column_name: &str,
        comment: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let tm = self
            .get_table_meta(table)
            .await?
            .ok_or_else(|| format!("Table '{}' not found", table))?;
        let col_prefix = format!("{}:{:020}:{}:", META_COLUMN_PREFIX, tm.table_id, column_name);
        let entries = self.scan_meta_prefix(col_prefix.as_bytes())?;
        for (key, value) in entries {
            let mut cm: ColumnMeta = <ColumnMeta as protobuf::Message>::parse_from_bytes(&value)?;
            if cm.column_name == column_name {
                cm.comment = comment.to_string();
                let db = self.db.write().unwrap();
                db.put(key, write_proto_to_vec(&cm))?;
                return Ok(());
            }
        }
        Err(format!("Column '{}' not found in table '{}'", column_name, table).into())
    }

    async fn put(
        &mut self,
        table: &str,
        key: &[u8],
        value: &[u8],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let cf_name = Self::get_cf_name(table);
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&cf_name)
            .ok_or_else(|| format!("Table '{}' not found", table))?;
        db.put_cf(cf, Self::key_to_bytes(key), value)?;
        Ok(())
    }

    async fn get(
        &self,
        table: &str,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        let cf_name = Self::get_cf_name(table);
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&cf_name)
            .ok_or_else(|| format!("Table '{}' not found", table))?;
        Ok(db.get_cf(cf, Self::key_to_bytes(key))?)
    }

    async fn delete(
        &mut self,
        table: &str,
        key: &[u8],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let cf_name = Self::get_cf_name(table);
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&cf_name)
            .ok_or_else(|| format!("Table '{}' not found", table))?;
        db.delete_cf(cf, Self::key_to_bytes(key))?;
        Ok(())
    }

    async fn query(
        &self,
        _query: &Query,
    ) -> Result<QueryResult, Box<dyn std::error::Error + Send + Sync>> {
        Err("structured query is not supported in KV engine; use get/scan methods instead".into())
    }

    fn get_schema_name(&self) -> &str {
        &self.schema_name
    }

    async fn shutdown(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("Shutting down KV RocksDB engine...");
        let db = self.db.write().unwrap();
        db.flush()?;
        info!("KV RocksDB engine flushed and shutdown complete");
        Ok(())
    }

    async fn scan_table(
        &self,
        _table: &str,
        _limit: Option<usize>,
    ) -> Result<Vec<(u64, Row)>, Box<dyn std::error::Error + Send + Sync>> {
        Err("scan_table is not supported in KV engine".into())
    }

    async fn get_column_types(
        &self,
        table: &str,
    ) -> Result<HashMap<String, ColumnType>, Box<dyn std::error::Error + Send + Sync>> {
        let cols = self.list_table_cols(table).await?;
        let mut map = HashMap::new();
        for c in cols {
            map.insert(c.column_name, c.column_type.enum_value_or(ColumnType::COLUMN_TYPE_STRING));
        }
        Ok(map)
    }
}
