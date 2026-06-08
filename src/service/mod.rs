use laoflchdb_engines::{StorageEngine, EngineOptions, SQLEngine, QueryResult, ColumnMeta, Query};
use laoflchdb_engines::{ColumnType, TableMeta, Row};
use std::sync::Arc;
use std::collections::HashMap;
use std::fmt;

pub struct SchemaManager {
    engines: tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::RwLock<Box<dyn StorageEngine + 'static>>>>>,
    base_path: String,
    engine_factory: Arc<dyn Fn(&EngineOptions) -> Result<Box<dyn StorageEngine + 'static>, Box<dyn std::error::Error + Send + Sync>> + Send + Sync>,
}

impl fmt::Debug for SchemaManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SchemaManager")
            .field("base_path", &self.base_path)
            .finish()
    }
}

impl SchemaManager {
    pub async fn new<F>(base_path: &str, engine_factory: F) -> Self
    where
        F: Fn(&EngineOptions) -> Result<Box<dyn StorageEngine>, Box<dyn std::error::Error + Send + Sync>> + Send + Sync + 'static,
    {
        Self {
            engines: tokio::sync::Mutex::new(HashMap::new()),
            base_path: base_path.to_string(),
            engine_factory: Arc::new(engine_factory),
        }
    }

    pub async fn new_with_engine(base_path: &str, default_schema: &str, engine: Arc<tokio::sync::RwLock<multi_table_rocksdb::MultiTableRocksDBEngine>>) -> Self {
        let mut engines: HashMap<String, Arc<tokio::sync::RwLock<Box<dyn StorageEngine + 'static>>>> = HashMap::new();
        engines.insert(default_schema.to_string(), Arc::new(tokio::sync::RwLock::new(Box::new(engine.read().await.clone()))));
        
        Self {
            engines: tokio::sync::Mutex::new(engines),
            base_path: base_path.to_string(),
            engine_factory: Arc::new(|options| {
                Ok(Box::new(multi_table_rocksdb::MultiTableRocksDBEngine::new(options)?))
            }),
        }
    }

    pub async fn get_schema_engine(&self, schema: &str) -> Result<Arc<tokio::sync::RwLock<Box<dyn StorageEngine + 'static>>>, Box<dyn std::error::Error + Send + Sync>> {
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
        let engine = (self.engine_factory)(&options)?;
        
        let mut engines = self.engines.lock().await;
        let engine = Arc::new(tokio::sync::RwLock::new(engine));
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
        let engine = (self.engine_factory)(&options)?;
        
        let mut engines = self.engines.lock().await;
        engines.insert(schema.to_string(), Arc::new(tokio::sync::RwLock::new(engine)));
        
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
    
    async fn create_table(&self, schema: &str, table: &str, columns: &[(u32, &str, ColumnType)]) -> Result<u64, Box<dyn std::error::Error + Send + Sync>>;
    async fn drop_table(&self, schema: &str, table: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    async fn list_tables(&self, schema: &str) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>>;
    async fn list_table_cols(&self, schema: &str, table: &str) -> Result<Vec<ColumnMeta>, Box<dyn std::error::Error + Send + Sync>>;
    
    async fn add_row(&self, schema: &str, table: &str, row: &Row) -> Result<u64, Box<dyn std::error::Error + Send + Sync>>;
    async fn get_row(&self, schema: &str, table: &str, row_id: u64) -> Result<Option<Row>, Box<dyn std::error::Error + Send + Sync>>;
    async fn delete_row(&self, schema: &str, table: &str, row_id: u64) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    async fn update_row(&self, schema: &str, table: &str, row_id: u64, row: &Row) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    async fn query(&self, schema: &str, query: &Query) -> Result<QueryResult, Box<dyn std::error::Error + Send + Sync>>;
    
    async fn get_all_meta(&self, schema: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>>;
    async fn get_schema_info(&self, schema: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>>;
    async fn get_table_meta(&self, schema: &str, table: &str) -> Result<Option<TableMeta>, Box<dyn std::error::Error + Send + Sync>>;
    
    async fn put(&self, schema: &str, table: &str, key: &[u8], value: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    async fn get(&self, schema: &str, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>>;
    async fn delete(&self, schema: &str, table: &str, key: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    async fn sql_query(&self, schema: &str, sql: &str) -> Result<QueryResult, Box<dyn std::error::Error + Send + Sync>>;
    
    async fn refresh_tables(&self, schema: &str) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>>;
}

pub struct DatabaseServiceImpl {
    schema_manager: Arc<SchemaManager>,
    sql_engine: Arc<tokio::sync::RwLock<dyn SQLEngine>>,
    default_schema: String,
}

impl DatabaseServiceImpl {
    pub async fn new_with_sql_engine(base_path: &str, sql_engine: Arc<tokio::sync::RwLock<dyn SQLEngine>>) -> Self {
        let schema_manager = Arc::new(SchemaManager::new(base_path, |options| {
            Ok(Box::new(multi_table_rocksdb::MultiTableRocksDBEngine::new(options)?))
        }).await);
        
        Self { 
            schema_manager,
            sql_engine,
            default_schema: "sys".to_string(),
        }
    }

    pub async fn new(base_path: &str) -> Self {
        use laoflchdb_sql_df_engine::DataFusionSQLEngine;

        let sys_engine = Arc::new(tokio::sync::RwLock::new(multi_table_rocksdb::MultiTableRocksDBEngine::new(&laoflchdb_engines::EngineOptions {
            db_path: format!("{}/sys", base_path),
            schema_name: "sys".to_string(),
        }).unwrap()));
        
        let mut df_engine = DataFusionSQLEngine::new(sys_engine.clone());
        df_engine.refresh_tables().await.unwrap();
        let sql_engine = Arc::new(tokio::sync::RwLock::new(df_engine));

        let schema_manager = Arc::new(SchemaManager::new_with_engine(base_path, "sys", sys_engine).await);
        
        Self { 
            schema_manager,
            sql_engine,
            default_schema: "sys".to_string(),
        }
    }

    pub fn schema_manager(&self) -> &Arc<SchemaManager> {
        &self.schema_manager
    }
    
    pub fn sql_engine(&self) -> &Arc<tokio::sync::RwLock<dyn SQLEngine>> {
        &self.sql_engine
    }
}

#[async_trait::async_trait]
impl DatabaseService for DatabaseServiceImpl {
    async fn init_database(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let sys_schema = "sys";
        
        let schemas = self.schema_manager.as_ref().list_schemas().await;
        let schema_exists = schemas.contains(&sys_schema.to_string());
        
        let existing_tables = self.list_tables(sys_schema).await.unwrap_or_default();
        let user_table_exists = existing_tables.contains(&"user".to_string());
        
        if schema_exists && user_table_exists {
            println!("✅ 数据库已初始化，本次启动不执行初始化");
            return Ok(());
        }
        
        if !schema_exists {
            println!("✅ 创建 Schema '{}'", sys_schema);
            self.schema_manager.as_ref().create_schema(sys_schema).await?;
        }
        
        if !user_table_exists {
            println!("✅ 创建表 'user'");
            let user_columns = [
                (1, "id", ColumnType::COLUMN_TYPE_INT64),
                (2, "username", ColumnType::COLUMN_TYPE_STRING),
                (3, "email", ColumnType::COLUMN_TYPE_STRING),
                (4, "password_hash", ColumnType::COLUMN_TYPE_STRING),
                (5, "created_at", ColumnType::COLUMN_TYPE_STRING),
            ];
            self.create_table(sys_schema, "user", &user_columns).await?;
        }
        
        Ok(())
    }

    async fn create_schema(&self, schema: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.schema_manager.as_ref().create_schema(schema).await
    }

    async fn list_schemas(&self) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        let mut schemas = vec![self.default_schema.clone()];
        let additional = self.schema_manager.as_ref().list_schemas().await;
        for s in additional {
            if s != self.default_schema {
                schemas.push(s);
            }
        }
        Ok(schemas)
    }

    async fn drop_schema(&self, schema: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.schema_manager.as_ref().drop_schema(schema).await
    }

    async fn create_table(&self, schema: &str, table: &str, columns: &[(u32, &str, ColumnType)]) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.as_ref().get_schema_engine(schema).await?;
        let table = table.to_string();
        let columns = columns.to_vec();
        let mut engine = engine.write().await;
        let table_id = engine.as_mut().create_table(&table, &columns).await?;
        
        if schema == "sys" {
            let mut sql_engine = self.sql_engine.write().await;
            sql_engine.register_table(&table).await?;
        }
        
        Ok(table_id)
    }

    async fn drop_table(&self, schema: &str, table: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.as_ref().get_schema_engine(schema).await?;
        let table = table.to_string();
        let mut engine = engine.write().await;
        engine.as_mut().drop_table(&table).await?;
        
        if schema == "sys" {
            let mut sql_engine = self.sql_engine.write().await;
            sql_engine.deregister_table(&table).await?;
        }
        
        Ok(())
    }

    async fn list_tables(&self, schema: &str) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.as_ref().get_schema_engine(schema).await?;
        let engine = engine.read().await;
        engine.as_ref().list_tables().await
    }

    async fn list_table_cols(&self, schema: &str, table: &str) -> Result<Vec<ColumnMeta>, Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.as_ref().get_schema_engine(schema).await?;
        let table = table.to_string();
        let engine = engine.read().await;
        engine.as_ref().list_table_cols(&table).await
    }

    async fn add_row(&self, schema: &str, table: &str, row: &Row) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.as_ref().get_schema_engine(schema).await?;
        let table = table.to_string();
        let row = row.clone();
        let mut engine = engine.write().await;
        engine.as_mut().add_row(&table, &row).await
    }

    async fn get_row(&self, schema: &str, table: &str, row_id: u64) -> Result<Option<Row>, Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.as_ref().get_schema_engine(schema).await?;
        let table = table.to_string();
        let engine = engine.read().await;
        engine.as_ref().get_row(&table, row_id).await
    }

    async fn delete_row(&self, schema: &str, table: &str, row_id: u64) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.as_ref().get_schema_engine(schema).await?;
        let table = table.to_string();
        let mut engine = engine.write().await;
        engine.as_mut().delete_row(&table, row_id).await
    }

    async fn update_row(&self, schema: &str, table: &str, row_id: u64, row: &Row) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.as_ref().get_schema_engine(schema).await?;
        let table = table.to_string();
        let row = row.clone();
        let mut engine = engine.write().await;
        engine.as_mut().update_row(&table, row_id, &row).await
    }

    async fn get_all_meta(&self, schema: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.as_ref().get_schema_engine(schema).await?;
        let engine = engine.read().await;
        engine.as_ref().get_all_meta().await
    }

    async fn get_schema_info(&self, schema: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.as_ref().get_schema_engine(schema).await?;
        let engine = engine.read().await;
        engine.as_ref().get_schema_info().await
    }

    async fn get_table_meta(&self, schema: &str, table: &str) -> Result<Option<TableMeta>, Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.as_ref().get_schema_engine(schema).await?;
        let table = table.to_string();
        let engine = engine.read().await;
        engine.as_ref().get_table_meta(&table).await
    }

    async fn put(&self, schema: &str, table: &str, key: &[u8], value: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.as_ref().get_schema_engine(schema).await?;
        let table = table.to_string();
        let key = key.to_vec();
        let value = value.to_vec();
        let mut engine = engine.write().await;
        engine.as_mut().put(&table, &key, &value).await
    }

    async fn get(&self, schema: &str, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.as_ref().get_schema_engine(schema).await?;
        let table = table.to_string();
        let key = key.to_vec();
        let engine = engine.read().await;
        engine.as_ref().get(&table, &key).await
    }

    async fn delete(&self, schema: &str, table: &str, key: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.as_ref().get_schema_engine(schema).await?;
        let table = table.to_string();
        let key = key.to_vec();
        let mut engine = engine.write().await;
        engine.as_mut().delete(&table, &key).await
    }

    async fn query(&self, schema: &str, query: &Query) -> Result<QueryResult, Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.as_ref().get_schema_engine(schema).await?;
        let mut engine = engine.write().await;
        engine.as_mut().query(query).await
    }
    
    async fn sql_query(&self, _schema: &str, sql: &str) -> Result<QueryResult, Box<dyn std::error::Error + Send + Sync>> {
        let sql_engine = self.sql_engine.read().await;
        sql_engine.execute_query(sql).await
    }
    
    async fn refresh_tables(&self, _schema: &str) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        let mut sql_engine = self.sql_engine.write().await;
        sql_engine.refresh_tables().await?;
        
        // 获取当前所有表列表（从 sys schema 中获取）
        let sys_engine = self.schema_manager.as_ref().get_schema_engine("sys").await?;
        let sys_engine = sys_engine.read().await;
        let tables = sys_engine.as_ref().list_tables().await?;
        
        Ok(tables)
    }
}
