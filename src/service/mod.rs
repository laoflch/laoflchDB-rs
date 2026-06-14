use laoflchdb_engines::{StorageEngine, EngineOptions, SQLEngine, QueryResult, ColumnMeta, Query};
use laoflchdb_engines::{ColumnType, TableMeta, Row, Field, SpecialFields, EnumOrUnknown, RowType};
use laoflchdb_engines::field::field::Value;
use laoflchdb_engines::Message;
use std::sync::Arc;
use std::collections::HashMap;
use std::fmt;
use sha2::{Sha256, Digest};
use chrono;

pub mod index;

pub use index::{IndexService,IndexServiceImpl};

// 密码哈希函数
fn hash_password(password: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}

// 字段编码函数
fn encode_field(f: &Field) -> Vec<u8> {
    let mut buf = Vec::new();
    f.write_to_vec(&mut buf).unwrap();
    buf
}

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
    pub fn get_base_path(&self) -> &str {
        &self.base_path
    }
    
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
        
        let path = std::path::Path::new(base_path);
        if path.exists() && path.is_dir() {
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    let dir_path = entry.path();
                    if dir_path.is_dir() {
                        if let Some(schema_name) = dir_path.file_name().and_then(|n| n.to_str()) {
                            if schema_name != default_schema && !engines.contains_key(schema_name) {
                                let schema_path = format!("{}/{}", base_path, schema_name);
                                let options = EngineOptions {
                                    db_path: schema_path,
                                    schema_name: schema_name.to_string(),
                                };
                                if let Ok(engine) = multi_table_rocksdb::MultiTableRocksDBEngine::new(&options) {
                                    engines.insert(schema_name.to_string(), Arc::new(tokio::sync::RwLock::new(Box::new(engine))));
                                }
                            }
                        }
                    }
                }
            }
        }
        
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
        
        std::fs::create_dir_all(&self.base_path)?;
        
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
    
    async fn create_table(&self, schema: &str, table: &str, table_comment: Option<&str>, columns: &[(u32, &str, ColumnType, Option<&str>)]) -> Result<u64, Box<dyn std::error::Error + Send + Sync>>;
    async fn drop_table(&self, schema: &str, table: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    async fn list_tables(&self, schema: &str) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>>;
    async fn list_table_cols(&self, schema: &str, table: &str) -> Result<Vec<ColumnMeta>, Box<dyn std::error::Error + Send + Sync>>;
    
    async fn update_table_comment(&self, schema: &str, table: &str, comment: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    async fn update_column_comment(&self, schema: &str, table: &str, column_name: &str, comment: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
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
    
    async fn shutdown(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
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
        
        // 启动时从 SchemaManager 获取所有已加载的 schema 并注册到 SQL 引擎
        {
            let mut sql_engine_guard = sql_engine.write().await;
            let schemas = schema_manager.list_schemas().await;
            log::info!("[SQL] Found {} schemas: {:?}", schemas.len(), schemas);
            for schema_name in schemas {
                if schema_name != "sys" {
                    log::info!("[SQL] Processing schema: {}", schema_name);
                    if let Ok(engine) = schema_manager.get_schema_engine(&schema_name).await {
                        let engine_read = engine.read().await;
                        if let Ok(tables) = engine_read.list_tables().await {
                            log::info!("[SQL] Found {} tables in schema '{}': {:?}", tables.len(), schema_name, tables);
                            for table in tables {
                                let full_table_name = format!("{}.{}", schema_name, table);
                                log::info!("[SQL] Registering table '{}'", full_table_name);
                                let table_provider = engine_read.create_table_provider(&full_table_name);
                                let result = sql_engine_guard.register_table_with_schema(&schema_name, &table, table_provider).await;
                                log::info!("[SQL] Registration result for '{}.{}': {:?}", schema_name, table, result);
                            }
                        } else {
                            log::warn!("[SQL] Failed to list tables in schema '{}'", schema_name);
                        }
                    } else {
                        log::warn!("[SQL] Failed to get engine for schema '{}'", schema_name);
                    }
                }
            }
        }
        
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
        
        // 检查 user 表结构是否为新版本 (id, username, email, password_hash, created_at)
        let user_table_needs_update = if user_table_exists {
            // 使用 list_table_cols 检查表结构，避免使用 SQL 查询
            match self.list_table_cols(sys_schema, "user").await {
                Ok(cols) => {
                    let col_names: Vec<String> = cols.iter().map(|c| c.column_name.clone()).collect();
                    // 新版本表应该包含 username 字段
                    !col_names.contains(&"username".to_string())
                }
                Err(_) => true,
            }
        } else {
            false
        };
        
        if schema_exists && user_table_exists && !user_table_needs_update {
            println!("✅ 数据库已初始化，本次启动不执行初始化");
            return Ok(());
        }
        
        if !schema_exists {
            println!("✅ 创建 Schema '{}'", sys_schema);
            self.schema_manager.as_ref().create_schema(sys_schema).await?;
        }

        // 如果 user 表不存在或结构需要更新，则删除旧表并创建新表
        if user_table_exists {
            println!("⚠️ 检测到旧版本 user 表，删除并重建...");
            self.drop_table(sys_schema, "user").await?;
        }
        
        println!("✅ 创建表 'user' (id, username, email, password_hash, created_at)");
        let user_columns = [
            (1, "id", ColumnType::COLUMN_TYPE_INT64, Some("用户ID，主键自增")),
            (2, "username", ColumnType::COLUMN_TYPE_STRING, Some("用户名，唯一标识")),
            (3, "email", ColumnType::COLUMN_TYPE_STRING, Some("邮箱地址")),
            (4, "password_hash", ColumnType::COLUMN_TYPE_STRING, Some("密码哈希值")),
            (5, "created_at", ColumnType::COLUMN_TYPE_STRING, Some("创建时间")),
        ];
        self.create_table(sys_schema, "user", Some("用户表：存储系统用户信息"), &user_columns).await?;

        // 创建默认用户
        println!("✅ 创建默认用户 'admin'");
        let admin_password_hash = hash_password("laoflchdb");
        let created_at = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();

        // 使用 add_row API 添加用户
        let id_field = Field {
            value: Some(Value::IntegerValue(laoflchdb_engines::field::Integer { 
                value: 1,
                special_fields: SpecialFields::default(),
            })),
            special_fields: SpecialFields::default(),
        };
        let username_field = Field {
            value: Some(Value::StringValue(laoflchdb_engines::field::String { 
                value: "admin".to_string(),
                special_fields: SpecialFields::default(),
            })),
            special_fields: SpecialFields::default(),
        };
        let email_field = Field {
            value: Some(Value::StringValue(laoflchdb_engines::field::String { 
                value: "admin@laoflchdb.local".to_string(),
                special_fields: SpecialFields::default(),
            })),
            special_fields: SpecialFields::default(),
        };
        let password_field = Field {
            value: Some(Value::StringValue(laoflchdb_engines::field::String { 
                value: admin_password_hash,
                special_fields: SpecialFields::default(),
            })),
            special_fields: SpecialFields::default(),
        };
        let created_at_field = Field {
            value: Some(Value::StringValue(laoflchdb_engines::field::String { 
                value: created_at,
                special_fields: SpecialFields::default(),
            })),
            special_fields: SpecialFields::default(),
        };

        let row = Row {
            row_type: EnumOrUnknown::new(RowType::ROW_TYPE_NORMAL),
            version: 1,
            data: vec![
                encode_field(&id_field),
                encode_field(&username_field),
                encode_field(&email_field),
                encode_field(&password_field),
                encode_field(&created_at_field),
            ],
            special_fields: SpecialFields::default(),
        };

        self.add_row(sys_schema, "user", &row).await?;
        println!("   提示: 默认用户名 'admin', 密码 'laoflchdb'");

        Ok(())
    }

    async fn create_schema(&self, schema: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.schema_manager.as_ref().create_schema(schema).await?;
        
        // 在 DataFusion 中创建对应的 schema
        let sql_engine = self.sql_engine.write().await;
        let create_schema_sql = format!("CREATE SCHEMA IF NOT EXISTS laoflchdb.{};", schema);
        log::info!("[SQL] Creating schema '{}' in DataFusion", schema);
        match sql_engine.execute_query(&create_schema_sql).await {
            Ok(_) => log::info!("[SQL] Schema '{}' created successfully in DataFusion", schema),
            Err(e) => log::warn!("[SQL] Failed to create schema '{}' in DataFusion: {}", schema, e),
        }
        
        Ok(())
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

    async fn create_table(&self, schema: &str, table: &str, table_comment: Option<&str>, columns: &[(u32, &str, ColumnType, Option<&str>)]) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        let engine_ref = self.schema_manager.as_ref().get_schema_engine(schema).await?;
        let table_name = table.to_string();
        let columns = columns.to_vec();
        
        log::warn!("[CreateTable] Creating table '{}.{}' with {} columns", schema, table_name, columns.len());
        
        let table_id = {
            let mut engine_write = engine_ref.write().await;
            engine_write.as_mut().create_table(&table_name, table_comment, &columns).await?
        };
        
        log::info!("[CreateTable] Table ID: {}", table_id);
        
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        
        // 验证表是否创建成功
        let engine_read = engine_ref.read().await;
        if let Ok(cols) = engine_read.list_table_cols(&table_name).await {
            log::info!("[CreateTable] Table '{}' columns after creation: {:?}", table_name, cols.iter().map(|c| c.column_name.clone()).collect::<Vec<_>>());
        } else {
            log::error!("[CreateTable] Failed to get columns for '{}'", table_name);
        }
        
        let full_table_name = format!("{}.{}", schema, table_name);
        log::info!("[SQL] Creating table provider for '{}'", full_table_name);
        
        let table_provider = engine_read.create_table_provider(&full_table_name);
        
        let mut sql_engine = self.sql_engine.write().await;
        sql_engine.register_table_with_schema(schema, &table_name, table_provider).await?;
        log::info!("[SQL] Table '{}.{}' registered in DataFusion", schema, table_name);
        
        Ok(table_id)
    }

    async fn drop_table(&self, schema: &str, table: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.as_ref().get_schema_engine(schema).await?;
        let table = table.to_string();
        let mut engine = engine.write().await;
        engine.as_mut().drop_table(&table).await?;
        
        let mut sql_engine = self.sql_engine.write().await;
        sql_engine.deregister_table_with_schema(schema, &table).await?;
        log::info!("[SQL] Table '{}.{}' deregistered from DataFusion", schema, table);
        
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

    async fn update_table_comment(&self, schema: &str, table: &str, comment: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.as_ref().get_schema_engine(schema).await?;
        let table = table.to_string();
        let mut engine = engine.write().await;
        engine.as_mut().update_table_comment(&table, comment).await
    }

    async fn update_column_comment(&self, schema: &str, table: &str, column_name: &str, comment: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let engine = self.schema_manager.as_ref().get_schema_engine(schema).await?;
        let table = table.to_string();
        let column_name = column_name.to_string();
        let mut engine = engine.write().await;
        engine.as_mut().update_column_comment(&table, &column_name, comment).await
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
    
    async fn sql_query(&self, schema: &str, sql: &str) -> Result<QueryResult, Box<dyn std::error::Error + Send + Sync>> {
        // 从 SQL 中提取所有 schema 引用
        let mut schemas_to_register = std::collections::HashSet::new();
        schemas_to_register.insert(self.default_schema.clone());
        schemas_to_register.insert(schema.to_string());
        
        let schema_pattern = regex::Regex::new(r"(\w+)\.(\w+)").unwrap();
        for cap in schema_pattern.captures_iter(sql) {
            if let Some(schema_capture) = cap.get(1) {
                schemas_to_register.insert(schema_capture.as_str().to_string());
            }
        }
        
        let mut sql_engine = self.sql_engine.write().await;
        
        // 注册缺失的 schema 和表
        for schema_name in &schemas_to_register {
            let engine = match self.schema_manager.as_ref().get_schema_engine(schema_name).await {
                Ok(e) => e,
                Err(_) => {
                    // 如果 schema 不存在，尝试创建引擎并注册表
                    let base_path = self.schema_manager.get_base_path();
                    let db_path = format!("{}/{}", base_path, schema_name);
                    if let Ok(engine) = multi_table_rocksdb::MultiTableRocksDBEngine::new(&laoflchdb_engines::EngineOptions {
                        db_path,
                        schema_name: schema_name.to_string(),
                    }) {
                        let engine = Arc::new(tokio::sync::RwLock::new(engine));
                        let engine_read = engine.read().await;
                        if let Ok(tables) = engine_read.list_tables().await {
                            for table in tables {
                                let full_table_name = format!("{}.{}", schema_name, table);
                                let table_provider = engine_read.create_table_provider(&full_table_name);
                                let _ = sql_engine.register_table_with_schema(schema_name, &table, table_provider);
                            }
                        }
                    }
                    continue;
                }
            };
            
            let engine_read = engine.read().await;
            if let Ok(tables) = engine_read.list_tables().await {
                for table in tables {
                    let full_table_name = format!("{}.{}", schema_name, table);
                    let table_provider = engine_read.create_table_provider(&full_table_name);
                    let _ = sql_engine.register_table_with_schema(schema_name, &table, table_provider);
                }
            }
        }
        
        let modified_sql = if !schema.is_empty() && schema != "sys" {
            let from_pattern = regex::Regex::new(r"(?i)(FROM|JOIN)\s+([a-zA-Z_][a-zA-Z0-9_]*)(\s|,|;|$)").unwrap();
            from_pattern.replace_all(sql, |caps: &regex::Captures| {
                let keyword = caps.get(1).unwrap().as_str();
                let table_name = caps.get(2).unwrap().as_str();
                let suffix = caps.get(3).unwrap().as_str();
                if table_name.contains('.') {
                    if table_name.contains("laoflchdb.") {
                        format!("{} {}{}", keyword, table_name, suffix)
                    } else {
                        format!("{} laoflchdb.{}{}", keyword, table_name, suffix)
                    }
                } else {
                    format!("{} laoflchdb.{}.{}{}", keyword, schema, table_name, suffix)
                }
            }).to_string()
        } else {
            let from_pattern = regex::Regex::new(r"(?i)(FROM|JOIN)\s+([a-zA-Z_][a-zA-Z0-9_]*)(\s|,|;|$)").unwrap();
            from_pattern.replace_all(sql, |caps: &regex::Captures| {
                let keyword = caps.get(1).unwrap().as_str();
                let table_name = caps.get(2).unwrap().as_str();
                let suffix = caps.get(3).unwrap().as_str();
                if table_name.contains('.') {
                    if table_name.contains("laoflchdb.") {
                        format!("{} {}{}", keyword, table_name, suffix)
                    } else {
                        format!("{} laoflchdb.{}{}", keyword, table_name, suffix)
                    }
                } else {
                    format!("{} laoflchdb.sys.{}{}", keyword, table_name, suffix)
                }
            }).to_string()
        };
        
        sql_engine.execute_query(&modified_sql).await
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
    
    async fn shutdown(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        log::info!("开始关闭数据库服务...");
        
        let schemas = self.schema_manager.as_ref().list_schemas().await;
        for schema in schemas {
            log::info!("关闭 schema: {}", schema);
            if let Ok(engine) = self.schema_manager.as_ref().get_schema_engine(&schema).await {
                let mut engine = engine.write().await;
                engine.as_mut().shutdown().await?;
            }
        }
        
        log::info!("数据库服务关闭完成");
        Ok(())
    }
}
