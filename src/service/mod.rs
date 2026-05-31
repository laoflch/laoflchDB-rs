use laoflchdb_db_engine::{DBEngine, EngineOptions};
use multi_table_rocksdb::MultiTableRocksDBEngine;
use laoflchdb_db_engine::pb::{ColumnType, TableMeta, Row};
use std::sync::Arc;
use std::collections::HashMap;

pub struct SchemaManager {
    engines: tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<MultiTableRocksDBEngine>>>>,
    base_path: String,
}

impl SchemaManager {
    pub async fn new(base_path: &str) -> Self {
        Self {
            engines: tokio::sync::Mutex::new(HashMap::new()),
            base_path: base_path.to_string(),
        }
    }

    pub async fn get_schema_engine(&self, schema: &str) -> Result<Arc<tokio::sync::Mutex<MultiTableRocksDBEngine>>, Box<dyn std::error::Error + Send + Sync>> {
        {
            let engines = self.engines.lock().await;
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
        
        let mut engines = self.engines.lock().await;
        let engine = Arc::new(tokio::sync::Mutex::new(engine));
        engines.insert(schema.to_string(), engine.clone());
        
        Ok(engine)
    }

    pub async fn list_schemas(&self) -> Vec<String> {
        let engines = self.engines.lock().await;
        engines.keys().cloned().collect()
    }

    pub async fn create_schema(&self, schema: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        {
            let engines = self.engines.lock().await;
            if engines.contains_key(schema) {
                return Err(format!("Schema '{}' already exists", schema).into());
            }
        }

        let schema_path = format!("{}/{}", self.base_path, schema);
        let options = EngineOptions {
            db_path: schema_path,
            schema_name: schema.to_string(),
        };
        let _ = MultiTableRocksDBEngine::new(&options)?;
        
        let mut engines = self.engines.lock().await;
        let engine = Arc::new(tokio::sync::Mutex::new(MultiTableRocksDBEngine::new(&options)?));
        engines.insert(schema.to_string(), engine);
        
        Ok(())
    }

    pub async fn drop_schema(&self, schema: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if schema == "sys" {
            return Err("Cannot drop reserved schema 'sys'".into());
        }

        let mut engines = self.engines.lock().await;
        engines.remove(schema)
            .ok_or_else(|| format!("Schema '{}' not found", schema))?;
        
        Ok(())
    }
}

#[async_trait::async_trait]
pub trait DatabaseService: Send + Sync + 'static {
    async fn init_database(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    async fn create_schema(&self, schema: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    async fn list_schemas(&self) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>>;
    async fn drop_schema(&self, schema: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    // 表管理
    async fn create_table(&self, schema: &str, table: &str, columns: &[(u32, &str, ColumnType)]) -> Result<u64, Box<dyn std::error::Error + Send + Sync>>;
    async fn drop_table(&self, schema: &str, table: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    async fn list_tables(&self, schema: &str) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>>;
    async fn list_table_cols(&self, schema: &str, table: &str) -> Result<Vec<laoflchdb_db_engine::pb::ColumnMeta>, Box<dyn std::error::Error + Send + Sync>>;
    
    // 行操作
    async fn add_row(&self, schema: &str, table: &str, row: &Row) -> Result<u64, Box<dyn std::error::Error + Send + Sync>>;
    async fn get_row(&self, schema: &str, table: &str, row_id: u64) -> Result<Option<Row>, Box<dyn std::error::Error + Send + Sync>>;
    async fn delete_row(&self, schema: &str, table: &str, row_id: u64) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    async fn update_row(&self, schema: &str, table: &str, row_id: u64, row: &Row) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    // 查询操作
    async fn query(&self, schema: &str, query: &laoflchdb_db_engine::pb::Query) -> Result<laoflchdb_db_engine::pb::QueryResult, Box<dyn std::error::Error + Send + Sync>>;
    
    // 元数据查询
    async fn get_all_meta(&self, schema: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>>;
    async fn get_schema_info(&self, schema: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>>;
    async fn get_table_meta(&self, schema: &str, table: &str) -> Result<Option<TableMeta>, Box<dyn std::error::Error + Send + Sync>>;
    
    // KV 操作
    async fn put(&self, schema: &str, table: &str, key: &[u8], value: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    async fn get(&self, schema: &str, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>>;
    async fn delete(&self, schema: &str, table: &str, key: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

pub struct DatabaseServiceImpl {
    schema_manager: Arc<SchemaManager>,
    default_schema: String,
}

impl DatabaseServiceImpl {
    pub async fn new(base_path: &str) -> Self {
        let schema_manager = SchemaManager::new(base_path).await;
        Self { 
            schema_manager: Arc::new(schema_manager),
            default_schema: "sys".to_string(),
        }
    }
}

#[async_trait::async_trait]
impl DatabaseService for DatabaseServiceImpl {
    async fn init_database(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _ = self.schema_manager.get_schema_engine(&self.default_schema).await?;
        Ok(())
    }

    async fn create_schema(&self, schema: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.schema_manager.create_schema(schema).await
    }

    async fn list_schemas(&self) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        let mut schemas = vec![self.default_schema.clone()];
        let additional = self.schema_manager.list_schemas().await;
        for s in additional {
            if s != self.default_schema {
                schemas.push(s);
            }
        }
        Ok(schemas)
    }

    async fn drop_schema(&self, schema: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.schema_manager.drop_schema(schema).await
    }

    // 表管理
    async fn create_table(&self, schema: &str, table: &str, columns: &[(u32, &str, ColumnType)]) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.get_schema_engine(schema).await?;
        let table = table.to_string();
        let columns = columns.to_vec();
        let mut engine = engine.lock().await;
        engine.create_table(&table, &columns).await
    }

    async fn drop_table(&self, schema: &str, table: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.get_schema_engine(schema).await?;
        let table = table.to_string();
        let mut engine = engine.lock().await;
        engine.drop_table(&table).await
    }

    async fn list_tables(&self, schema: &str) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.get_schema_engine(schema).await?;
        let engine = engine.lock().await;
        engine.list_tables().await
    }

    async fn list_table_cols(&self, schema: &str, table: &str) -> Result<Vec<laoflchdb_db_engine::pb::ColumnMeta>, Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.get_schema_engine(schema).await?;
        let table = table.to_string();
        let engine = engine.lock().await;
        engine.list_table_cols(&table).await
    }

    // 行操作
    async fn add_row(&self, schema: &str, table: &str, row: &Row) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.get_schema_engine(schema).await?;
        let table = table.to_string();
        let row = row.clone();
        let mut engine = engine.lock().await;
        engine.add_row(&table, &row).await
    }

    async fn get_row(&self, schema: &str, table: &str, row_id: u64) -> Result<Option<Row>, Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.get_schema_engine(schema).await?;
        let table = table.to_string();
        let engine = engine.lock().await;
        engine.get_row(&table, row_id).await
    }

    async fn delete_row(&self, schema: &str, table: &str, row_id: u64) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.get_schema_engine(schema).await?;
        let table = table.to_string();
        let mut engine = engine.lock().await;
        engine.delete_row(&table, row_id).await
    }

    async fn update_row(&self, schema: &str, table: &str, row_id: u64, row: &Row) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.get_schema_engine(schema).await?;
        let table = table.to_string();
        let row = row.clone();
        let mut engine = engine.lock().await;
        engine.update_row(&table, row_id, &row).await
    }

    // 元数据查询
    async fn get_all_meta(&self, schema: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.get_schema_engine(schema).await?;
        let engine = engine.lock().await;
        engine.get_all_meta().await
    }

    async fn get_schema_info(&self, schema: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.get_schema_engine(schema).await?;
        let engine = engine.lock().await;
        engine.get_schema_info().await
    }

    async fn get_table_meta(&self, schema: &str, table: &str) -> Result<Option<TableMeta>, Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.get_schema_engine(schema).await?;
        let table = table.to_string();
        let engine = engine.lock().await;
        engine.get_table_meta(&table).await
    }

    // KV 操作
    async fn put(&self, schema: &str, table: &str, key: &[u8], value: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.get_schema_engine(schema).await?;
        let table = table.to_string();
        let key = key.to_vec();
        let value = value.to_vec();
        let mut engine = engine.lock().await;
        engine.put(&table, &key, &value).await
    }

    async fn get(&self, schema: &str, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.get_schema_engine(schema).await?;
        let table = table.to_string();
        let key = key.to_vec();
        let engine = engine.lock().await;
        engine.get(&table, &key).await
    }

    async fn delete(&self, schema: &str, table: &str, key: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.get_schema_engine(schema).await?;
        let table = table.to_string();
        let key = key.to_vec();
        let mut engine = engine.lock().await;
        engine.delete(&table, &key).await
    }

    async fn query(&self, schema: &str, query: &laoflchdb_db_engine::pb::Query) -> Result<laoflchdb_db_engine::pb::QueryResult, Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.get_schema_engine(schema).await?;
        let mut engine = engine.lock().await;
        engine.query(query).await
    }
}
