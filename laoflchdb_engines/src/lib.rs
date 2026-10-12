use std::sync::Arc;

pub mod field;
pub mod metadata;
pub mod query;
pub mod row;

pub use metadata::{SchemaMeta, TableMeta, ColumnMeta, ColumnType};
pub use row::Row;
pub use query::{QueryResult, QueryRow};
pub use query::{Query, FilterOperator, ColumnFilter, ColumnFilterCondition, TableFilter};
pub use field::Field;

pub use protobuf::{SpecialFields, EnumOrUnknown, Message};
pub use row::RowType;

pub struct EngineOptions {
    pub db_path: String,
    pub schema_name: String,
}

pub const META_SCHEMA_PREFIX: &str = "META-SCHEMA";
pub const META_TABLE_PREFIX: &str = "META-TABLE";
pub const META_COLUMN_PREFIX: &str = "META-COL";
pub const MAX_TABLE_ID_LENGTH: usize = 20;

#[async_trait::async_trait]
pub trait StorageEngine: Send + Sync + 'static {
    async fn create_table(&mut self, table: &str, table_comment: Option<&str>, columns: &[(u32, &str, ColumnType, Option<&str>)]) -> Result<u64, Box<dyn std::error::Error + Send + Sync>>;
    async fn drop_table(&mut self, table: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    async fn list_tables(&self) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>>;
    async fn list_table_cols(&self, table: &str) -> Result<Vec<ColumnMeta>, Box<dyn std::error::Error + Send + Sync>>;
    
    async fn add_row(&mut self, table: &str, row: &Row, doc_id: Option<&str>) -> Result<u64, Box<dyn std::error::Error + Send + Sync>>;
    async fn get_row(&self, table: &str, row_id: u64) -> Result<Option<Row>, Box<dyn std::error::Error + Send + Sync>>;
    async fn delete_row(&mut self, table: &str, row_id: u64) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    async fn update_row(&mut self, table: &str, row_id: u64, row: &Row) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    async fn get_all_meta(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>>;
    async fn get_schema_info(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>>;
    async fn get_table_meta(&self, table: &str) -> Result<Option<TableMeta>, Box<dyn std::error::Error + Send + Sync>>;
    
    async fn update_table_comment(&mut self, table: &str, comment: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    async fn update_column_comment(&mut self, table: &str, column_name: &str, comment: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    async fn put(&mut self, table: &str, key: &[u8], value: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    async fn get(&self, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>>;
    async fn delete(&mut self, table: &str, key: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    async fn query(&self, query: &Query) -> Result<QueryResult, Box<dyn std::error::Error + Send + Sync>>;
    
    fn get_schema_name(&self) -> &str;
    
    async fn shutdown(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }
    
    async fn scan_table(&self, table: &str, offset: Option<usize>, limit: Option<usize>) -> Result<Vec<(u64, Row)>, Box<dyn std::error::Error + Send + Sync>>;
    
    async fn get_column_types(&self, table: &str) -> Result<std::collections::HashMap<String, ColumnType>, Box<dyn std::error::Error + Send + Sync>>;
    
    fn create_table_provider(&self, _table_name: &str) -> Arc<dyn datafusion::datasource::TableProvider> {
        unimplemented!()
    }
}

#[async_trait::async_trait]
pub trait SQLEngine: Send + Sync + 'static {
    async fn execute_query(&self, sql: &str) -> Result<QueryResult, Box<dyn std::error::Error + Send + Sync>>;
    
    async fn register_table(&mut self, table_name: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    async fn register_table_with_schema(&mut self, schema: &str, table_name: &str, table_provider: Arc<dyn datafusion::datasource::TableProvider>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _ = schema;
        let _ = table_name;
        let _ = table_provider;
        Ok(())
    }
    
    async fn deregister_table(&mut self, table_name: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    async fn deregister_table_with_schema(&mut self, schema: &str, table_name: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _ = schema;
        let _ = table_name;
        Ok(())
    }
    
    async fn refresh_tables(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

/// 面向 KV 的存储引擎接口（Raft 业务数据落库抽象）。
///
/// table 概念对应 Raft 服务的 group：每个 Raft 组的数据存放在
/// table = "group_{group_id}" 下。实现者内部自行保证线程安全，
/// 所有方法均为 &self（内部使用锁）。
#[async_trait::async_trait]
pub trait KvStorage: Send + Sync + 'static {
    /// 引擎类型名（如 "rocksdb"、"memory"）
    fn engine_type(&self) -> &str;

    /// 确保表存在（不存在则创建；重复调用幂等）
    async fn ensure_table(&self, table: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// 写入键值对
    async fn put(&self, table: &str, key: &[u8], value: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// 读取键值
    async fn get(&self, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>>;

    /// 删除键
    async fn delete(&self, table: &str, key: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// 全量扫描表内所有键值对
    async fn scan(&self, table: &str) -> Result<Vec<(Vec<u8>, Vec<u8>)>, Box<dyn std::error::Error + Send + Sync>>;

    /// 清空表内全部数据
    async fn clear(&self, table: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// 将数据刷新到持久介质（内存引擎可空实现）
    async fn flush(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }
}


