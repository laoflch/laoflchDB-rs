pub mod pb {
    include!(concat!(env!("OUT_DIR"), "/laoflchdb.metadata.rs"));
    include!(concat!(env!("OUT_DIR"), "/laoflchdb.row.rs"));
    include!(concat!(env!("OUT_DIR"), "/laoflchdb.field.rs"));
}

pub const META_SCHEMA_PREFIX: &str = "META-SCHEMA";
pub const META_TABLE_PREFIX: &str = "META-TABLE";
pub const META_COLUMN_PREFIX: &str = "META-COL";

pub trait DBEngine: Send + Sync + 'static {
    fn create_table(&mut self, table: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    fn drop_table(&mut self, table: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    fn list_tables(&self) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>>;
    
    fn put(&self, table: &str, key: &[u8], value: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    fn get(&self, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>>;
    fn delete(&self, table: &str, key: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    fn put_meta(&self, key: &[u8], value: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    fn get_meta(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>>;
    fn scan_meta_prefix(&self, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>, Box<dyn std::error::Error + Send + Sync>>;
    fn delete_meta(&self, key: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
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

pub use pb::{SchemaMeta, TableMeta, ColumnMeta, ColumnType};
