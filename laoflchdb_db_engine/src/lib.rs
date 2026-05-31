pub mod pb {
    include!(concat!(env!("OUT_DIR"), "/laoflchdb.rs"));
}

pub const META_SCHEMA_PREFIX: &str = "META-SCHEMA";
pub const META_TABLE_PREFIX: &str = "META-TABLE";
pub const META_COLUMN_PREFIX: &str = "META-COL";
pub const MAX_TABLE_ID_LENGTH: usize = 20;

#[async_trait::async_trait]
pub trait DBEngine: Send + Sync + 'static {
    // 表管理
    async fn create_table(&mut self, table: &str, columns: &[(u32, &str, pb::ColumnType)]) -> Result<u64, Box<dyn std::error::Error + Send + Sync>>;
    async fn drop_table(&mut self, table: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    async fn list_tables(&self) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>>;
    async fn list_table_cols(&self, table: &str) -> Result<Vec<pb::ColumnMeta>, Box<dyn std::error::Error + Send + Sync>>;
    
    // 行操作
    async fn add_row(&mut self, table: &str, row: &pb::Row) -> Result<u64, Box<dyn std::error::Error + Send + Sync>>;
    async fn get_row(&self, table: &str, row_id: u64) -> Result<Option<pb::Row>, Box<dyn std::error::Error + Send + Sync>>;
    async fn delete_row(&mut self, table: &str, row_id: u64) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    async fn update_row(&mut self, table: &str, row_id: u64, row: &pb::Row) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    // 元数据查询
    async fn get_all_meta(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>>;
    async fn get_schema_info(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>>;
    async fn get_table_meta(&self, table: &str) -> Result<Option<pb::TableMeta>, Box<dyn std::error::Error + Send + Sync>>;
    
    // KV 操作
    async fn put(&mut self, table: &str, key: &[u8], value: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    async fn get(&self, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>>;
    async fn delete(&mut self, table: &str, key: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    // Query 操作
    async fn query(&self, query: &pb::Query) -> Result<pb::QueryResult, Box<dyn std::error::Error + Send + Sync>>;
    
    fn get_schema_name(&self) -> &str;
}

pub struct EngineOptions {
    pub db_path: String,
    pub schema_name: String,
}

impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            db_path: "./db_data".to_string(),
            schema_name: "sys".to_string(),
        }
    }
}

pub use pb::{SchemaMeta, TableMeta, ColumnMeta, ColumnType, Row, RowType, Query, QueryResult, QueryRow,
               FilterOperator, ColumnFilter, ColumnFilterCondition, TableFilter};
