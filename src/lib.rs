pub mod db_engine;
pub mod service;
pub mod access;
pub mod server;
pub mod cli;
pub mod config;
pub mod init_data;

pub mod pb {
    pub mod rpc {
        tonic::include_proto!("laoflchdb.rpc");
    }
}

pub use db_engine::{
    StorageEngine, EngineOptions, MultiTableRocksDBEngine, MAX_TABLE_ID_LENGTH,
    SchemaMeta, TableMeta, ColumnMeta, ColumnType,
    META_SCHEMA_PREFIX, META_TABLE_PREFIX, META_COLUMN_PREFIX
};
pub use laoflchdb_engines::{SQLEngine, DataFusionStorageEngine, QueryResult};
pub use laoflchdb_sql_df_engine::DataFusionSQLEngine;
pub use service::{DatabaseService, DatabaseServiceImpl, SchemaManager};
pub use access::{AccessService, GrpcService, RestService};
pub use server::LaoflchDBServer;
pub use cli::{Cli, Commands};
pub use config::DatabaseConfig;

pub mod engine_factory {
    use std::sync::Arc;
    use super::{SQLEngine, DataFusionSQLEngine, EngineOptions, MultiTableRocksDBEngine, DatabaseServiceImpl};

    pub async fn create_default_database_service(db_path: &str) -> Result<Arc<DatabaseServiceImpl>, Box<dyn std::error::Error + Send + Sync>> {
        let sys_engine = MultiTableRocksDBEngine::new(&EngineOptions {
            db_path: format!("{}/sys", db_path),
            schema_name: "sys".to_string(),
        })?;
        let storage_engine = Arc::new(tokio::sync::RwLock::new(sys_engine));
        let df_engine = DataFusionSQLEngine::new(Arc::clone(&storage_engine));
        let sql_engine = Arc::new(tokio::sync::RwLock::new(df_engine));
        let service = DatabaseServiceImpl::new_with_sql_engine(db_path, sql_engine).await;
        Ok(Arc::new(service))
    }
}
