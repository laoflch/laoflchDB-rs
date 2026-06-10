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
pub use laoflchdb_engines::{SQLEngine, QueryResult};
pub use laoflchdb_sql_df_engine::{DataFusionSQLEngine, DataFusionStorageEngine};
pub use service::{DatabaseService, DatabaseServiceImpl, SchemaManager};
pub use access::{AccessService, GrpcService, RestService};
pub use server::LaoflchDBServer;
pub use cli::{Cli, Commands};
pub use config::{DatabaseConfig, RuntimeMode};

pub mod engine_factory {
    use std::sync::Arc;
    use super::DatabaseServiceImpl;

    pub async fn create_default_database_service(db_path: &str) -> Result<Arc<DatabaseServiceImpl>, Box<dyn std::error::Error + Send + Sync>> {
        let service = DatabaseServiceImpl::new(db_path).await;
        Ok(Arc::new(service))
    }
}
