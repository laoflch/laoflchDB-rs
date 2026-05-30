use crate::db_engine::{DBEngine, EngineOptions, MultiTableRocksDBEngine, MAX_TABLE_ID_LENGTH};
use crate::db_engine::pb::{ColumnType, SchemaMeta, TableMeta, ColumnMeta};
use crate::{META_SCHEMA_PREFIX, META_TABLE_PREFIX, META_COLUMN_PREFIX};
use prost::Message;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;

pub struct SchemaManager {
    engines: Mutex<HashMap<String, Arc<Mutex<MultiTableRocksDBEngine>>>>,
    base_path: String,
}

impl SchemaManager {
    pub fn new(base_path: &str) -> Self {
        Self {
            engines: Mutex::new(HashMap::new()),
            base_path: base_path.to_string(),
        }
    }

    pub fn get_schema_engine(&self, schema: &str) -> Result<Arc<Mutex<MultiTableRocksDBEngine>>, Box<dyn std::error::Error + Send + Sync>> {
        {
            let engines = self.engines.lock().unwrap();
            if let Some(engine) = engines.get(schema) {
                return Ok(engine.clone());
            }
        }

        let schema_path = format!("{}/{}", self.base_path, schema);
        let options = EngineOptions {
            db_path: schema_path,
            schema_name: schema.to_string(),
        };
        let engine = MultiTableRocksDBEngine::new(&options)?;
        
        let mut engines = self.engines.lock().unwrap();
        let engine = Arc::new(Mutex::new(engine));
        engines.insert(schema.to_string(), engine.clone());
        
        Ok(engine)
    }

    pub fn list_schemas(&self) -> Vec<String> {
        let engines = self.engines.lock().unwrap();
        engines.keys().cloned().collect()
    }

    pub fn create_schema(&self, schema: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        {
            let engines = self.engines.lock().unwrap();
            if engines.contains_key(schema) {
                return Err(format!("Schema '{}' already exists", schema).into());
            }
        }

        let schema_path = format!("{}/{}", self.base_path, schema);
        let options = EngineOptions {
            db_path: schema_path,
            schema_name: schema.to_string(),
        };
        let engine = MultiTableRocksDBEngine::new(&options)?;
        
        let mut engines = self.engines.lock().unwrap();
        engines.insert(schema.to_string(), Arc::new(Mutex::new(engine)));
        
        Ok(())
    }

    pub fn drop_schema(&self, schema: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if schema == "sys" {
            return Err("Cannot drop reserved schema 'sys'".into());
        }

        let mut engines = self.engines.lock().unwrap();
        engines.remove(schema)
            .ok_or_else(|| format!("Schema '{}' not found", schema))?;
        
        Ok(())
    }
}

pub trait DatabaseService: Send + Sync + 'static {
    fn init_database(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    fn create_schema(&self, schema: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    fn list_schemas(&self) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>>;
    fn drop_schema(&self, schema: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    fn put(&self, schema: &str, table: &str, key: &[u8], value: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    fn get(&self, schema: &str, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>>;
    fn delete(&self, schema: &str, table: &str, key: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    fn create_table(&self, schema: &str, table: &str, columns: &[(u32, &str, ColumnType)]) -> Result<u64, Box<dyn std::error::Error + Send + Sync>>;
    fn list_tables(&self, schema: &str) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>>;
    fn get_table_meta(&self, schema: &str, table: &str) -> Result<Option<TableMeta>, Box<dyn std::error::Error + Send + Sync>>;
}

pub struct DatabaseServiceImpl {
    schema_manager: Arc<SchemaManager>,
    default_schema: String,
}

impl DatabaseServiceImpl {
    pub fn new(schema_manager: Arc<SchemaManager>) -> Self {
        Self { 
            schema_manager,
            default_schema: "sys".to_string(),
        }
    }
    
    fn make_schema_meta_key(schema_name: &str) -> String {
        format!("{}:{}", META_SCHEMA_PREFIX, schema_name)
    }
    
    fn make_table_meta_key(table_name: &str, table_id: u64) -> String {
        format!("{}:{}:{}", META_TABLE_PREFIX, table_name, table_id)
    }
    
    fn format_table_id(table_id: u64) -> String {
        format!("{:0>width$}", table_id, width = MAX_TABLE_ID_LENGTH)
    }
    
    fn get_column_type_name(column_type: ColumnType) -> &'static str {
        match column_type {
            ColumnType::String => "COLUMN_TYPE_STRING",
            ColumnType::Int64 => "COLUMN_TYPE_INT64",
            ColumnType::Bytes => "COLUMN_TYPE_BYTES",
            ColumnType::Float => "COLUMN_TYPE_FLOAT",
            ColumnType::List => "COLUMN_TYPE_LIST",
            ColumnType::Image => "COLUMN_TYPE_IMAGE",
        }
    }
    
    fn make_column_meta_key(table_id: u64, column_name: &str, column_id: u64, column_type: ColumnType) -> String {
        let formatted_table_id = Self::format_table_id(table_id);
        let type_name = Self::get_column_type_name(column_type);
        format!("{}:{}:{}:{}:{}", META_COLUMN_PREFIX, formatted_table_id, column_name, column_id, type_name)
    }
}

impl DatabaseService for DatabaseServiceImpl {
    fn init_database(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.get_schema_engine(&self.default_schema)?;
        let mut engine = engine.lock().unwrap();
        
        let schema_meta_key = Self::make_schema_meta_key(&self.default_schema);
        let schema_meta_data = engine.get_meta(schema_meta_key.as_bytes())?;
        
        if schema_meta_data.is_none() {
            let schema_meta = SchemaMeta {
                schema_name: self.default_schema.clone(),
                next_auto_inc_table_id: 0,
            };
            let encoded = schema_meta.encode_to_vec();
            engine.put_meta(schema_meta_key.as_bytes(), &encoded)?;
            
            engine.create_table("user")?;
            let user_table_id: u64 = 0;
            let table_meta_key = Self::make_table_meta_key("user", user_table_id);
            let table_meta = TableMeta {
                table_id: user_table_id,
                table_name: "user".to_string(),
                column_count: 2,
                next_auto_inc_column_id: 2,
            };
            let encoded = table_meta.encode_to_vec();
            engine.put_meta(table_meta_key.as_bytes(), &encoded)?;
            
            let col_meta_key1 = Self::make_column_meta_key(user_table_id, "user_id", 0, ColumnType::Int64);
            let col_meta1 = ColumnMeta {
                table_id: user_table_id,
                column_id: 0,
                column_name: "user_id".to_string(),
                column_type: ColumnType::Int64.into(),
            };
            let encoded = col_meta1.encode_to_vec();
            engine.put_meta(col_meta_key1.as_bytes(), &encoded)?;
            
            let col_meta_key2 = Self::make_column_meta_key(user_table_id, "password", 1, ColumnType::String);
            let col_meta2 = ColumnMeta {
                table_id: user_table_id,
                column_id: 1,
                column_name: "password".to_string(),
                column_type: ColumnType::String.into(),
            };
            let encoded = col_meta2.encode_to_vec();
            engine.put_meta(col_meta_key2.as_bytes(), &encoded)?;
        }
        Ok(())
    }

    fn create_schema(&self, schema: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.schema_manager.create_schema(schema)?;
        let engine = self.schema_manager.get_schema_engine(schema)?;
        let mut engine = engine.lock().unwrap();
        
        let schema_meta_key = Self::make_schema_meta_key(schema);
        let schema_meta = SchemaMeta {
            schema_name: schema.to_string(),
            next_auto_inc_table_id: 0,
        };
        let encoded = schema_meta.encode_to_vec();
        engine.put_meta(schema_meta_key.as_bytes(), &encoded)?;
        
        Ok(())
    }

    fn list_schemas(&self) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        let mut schemas = vec![self.default_schema.clone()];
        let additional = self.schema_manager.list_schemas();
        for s in additional {
            if s != self.default_schema {
                schemas.push(s);
            }
        }
        Ok(schemas)
    }

    fn drop_schema(&self, schema: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.schema_manager.drop_schema(schema)
    }

    fn put(&self, schema: &str, table: &str, key: &[u8], value: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.get_schema_engine(schema)?;
        let engine = engine.lock().unwrap();
        engine.put(table, key, value)
    }

    fn get(&self, schema: &str, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.get_schema_engine(schema)?;
        let engine = engine.lock().unwrap();
        engine.get(table, key)
    }

    fn delete(&self, schema: &str, table: &str, key: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.get_schema_engine(schema)?;
        let engine = engine.lock().unwrap();
        engine.delete(table, key)
    }

    fn create_table(&self, schema: &str, table: &str, columns: &[(u32, &str, ColumnType)]) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.get_schema_engine(schema)?;
        let mut engine = engine.lock().unwrap();
        
        let schema_meta_key = Self::make_schema_meta_key(schema);
        let schema_meta_data = engine.get_meta(schema_meta_key.as_bytes())?;
        
        let mut next_table_id: u64 = 0;
        if let Some(data) = schema_meta_data {
            let mut schema_meta: SchemaMeta = prost::Message::decode(&data[..])?;
            next_table_id = schema_meta.next_auto_inc_table_id;
            schema_meta.next_auto_inc_table_id = next_table_id + 1;
            let encoded = schema_meta.encode_to_vec();
            engine.put_meta(schema_meta_key.as_bytes(), &encoded)?;
        } else {
            let schema_meta = SchemaMeta {
                schema_name: schema.to_string(),
                next_auto_inc_table_id: 1,
            };
            let encoded = schema_meta.encode_to_vec();
            engine.put_meta(schema_meta_key.as_bytes(), &encoded)?;
        }

        engine.create_table(table)?;

        let table_meta_key = Self::make_table_meta_key(table, next_table_id);
        let table_meta = TableMeta {
            table_id: next_table_id,
            table_name: table.to_string(),
            column_count: columns.len() as u32,
            next_auto_inc_column_id: columns.len() as u64,
        };
        let encoded = table_meta.encode_to_vec();
        engine.put_meta(table_meta_key.as_bytes(), &encoded)?;

        for (idx, (_, col_name, col_type)) in columns.iter().enumerate() {
            let col_id = idx as u64;
            let col_meta_key = Self::make_column_meta_key(next_table_id, col_name, col_id, *col_type);
            let col_meta = ColumnMeta {
                table_id: next_table_id,
                column_id: col_id,
                column_name: col_name.to_string(),
                column_type: (*col_type).into(),
            };
            let encoded = col_meta.encode_to_vec();
            engine.put_meta(col_meta_key.as_bytes(), &encoded)?;
        }

        Ok(next_table_id)
    }

    fn list_tables(&self, schema: &str) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.get_schema_engine(schema)?;
        let engine = engine.lock().unwrap();
        engine.list_tables()
    }

    fn get_table_meta(&self, schema: &str, table: &str) -> Result<Option<TableMeta>, Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.get_schema_engine(schema)?;
        let engine = engine.lock().unwrap();
        let prefix = format!("{}:{}:", META_TABLE_PREFIX, table);
        let meta_entries = engine.scan_meta_prefix(prefix.as_bytes())?;
        
        for (_, value) in meta_entries {
            let table_meta: TableMeta = prost::Message::decode(&value[..])?;
            return Ok(Some(table_meta));
        }
        
        Ok(None)
    }
}
