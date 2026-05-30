use rocksdb::{DB, Options, ColumnFamilyDescriptor};

use crate::{DBEngine, EngineOptions, META_TABLE_PREFIX};

pub struct MultiTableRocksDBEngine {
    db: DB,
    #[allow(dead_code)]
    schema_name: String,
}

impl MultiTableRocksDBEngine {
    pub fn new(options: &EngineOptions) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let path = &options.db_path;
        let schema_name = options.schema_name.clone();
        
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        let cf_descriptors = vec![
            ColumnFamilyDescriptor::new("default", Options::default()),
        ];

        let db = DB::open_cf_descriptors(&opts, path, cf_descriptors)?;
        Ok(Self { db, schema_name })
    }

    pub fn db_path(&self) -> String {
        self.db.path().to_string_lossy().to_string()
    }

    fn get_table_cf(&self, table: &str) -> String {
        table.to_string()
    }
}

impl DBEngine for MultiTableRocksDBEngine {
    fn create_table(&mut self, table: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if table == "default" {
            return Err("Cannot create reserved table 'default'".into());
        }
        
        let cf_name = self.get_table_cf(table);
        
        if self.db.cf_handle(&cf_name).is_some() {
            return Err(format!("Table '{}' already exists", cf_name).into());
        }
        
        let cf_opts = Options::default();
        self.db.create_cf(&cf_name, &cf_opts)?;
        
        Ok(())
    }

    fn drop_table(&mut self, table: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let prefix = format!("{}:{}:", META_TABLE_PREFIX, table);
        let entries = self.scan_meta_prefix(prefix.as_bytes())?;
        
        for (key, _) in entries {
            self.delete_meta(&key)?;
        }
        
        Ok(())
    }

    fn list_tables(&self) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        let prefix = format!("{}:", META_TABLE_PREFIX);
        let entries = self.scan_meta_prefix(prefix.as_bytes())?;
        
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

    fn put(&self, table: &str, key: &[u8], value: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let cf_name = self.get_table_cf(table);
        let cf_handle = self.db.cf_handle(&cf_name)
            .ok_or_else(|| format!("Table '{}' not found", cf_name))?;
        self.db.put_cf(cf_handle, key, value)?;
        Ok(())
    }

    fn get(&self, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        let cf_name = self.get_table_cf(table);
        let cf_handle = self.db.cf_handle(&cf_name)
            .ok_or_else(|| format!("Table '{}' not found", cf_name))?;
        let result = self.db.get_cf(cf_handle, key)?;
        Ok(result)
    }

    fn delete(&self, table: &str, key: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let cf_name = self.get_table_cf(table);
        let cf_handle = self.db.cf_handle(&cf_name)
            .ok_or_else(|| format!("Table '{}' not found", cf_name))?;
        self.db.delete_cf(cf_handle, key)?;
        Ok(())
    }

    fn put_meta(&self, key: &[u8], value: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let cf_handle = self.db.cf_handle("default")
            .ok_or_else(|| "Default column family not found".to_string())?;
        self.db.put_cf(cf_handle, key, value)?;
        Ok(())
    }

    fn get_meta(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        let cf_handle = self.db.cf_handle("default")
            .ok_or_else(|| "Default column family not found".to_string())?;
        let result = self.db.get_cf(cf_handle, key)?;
        Ok(result)
    }

    fn scan_meta_prefix(&self, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>, Box<dyn std::error::Error + Send + Sync>> {
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

    fn delete_meta(&self, key: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let cf_handle = self.db.cf_handle("default")
            .ok_or_else(|| "Default column family not found".to_string())?;
        self.db.delete_cf(cf_handle, key)?;
        Ok(())
    }

    fn get_schema_name(&self) -> &str {
        &self.schema_name
    }
}
