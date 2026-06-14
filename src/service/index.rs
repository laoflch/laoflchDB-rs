use std::sync::Arc;
use std::collections::HashMap;
use log::{info, debug, warn};

use laoflchdb_engines::{ColumnType, ColumnMeta, TableMeta, StorageEngine};
use index_tantivy::TantivyStorageEngine;

/// 全文索引服务 trait
#[async_trait::async_trait]
pub trait IndexService: Send + Sync + 'static {
    /// 初始化索引服务
    async fn init(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    /// 创建索引（类似于表）
    async fn create_index(
        &self, 
        index_name: &str, 
        fields: &[(u32, &str, ColumnType, Option<&str>)]
    ) -> Result<u64, Box<dyn std::error::Error + Send + Sync>>;
    
    /// 删除索引
    async fn drop_index(&self, index_name: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    /// 列出所有索引
    async fn list_indices(&self) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>>;
    
    /// 获取索引的字段信息
    async fn get_index_fields(&self, index_name: &str) -> Result<Vec<ColumnMeta>, Box<dyn std::error::Error + Send + Sync>>;
    
    /// 获取索引元数据
    async fn get_index_meta(&self, index_name: &str) -> Result<Option<TableMeta>, Box<dyn std::error::Error + Send + Sync>>;
    
    /// 添加文档到索引
    async fn add_document(
        &self, 
        index_name: &str, 
        doc_id: &str,
        fields: HashMap<String, String>
    ) -> Result<u64, Box<dyn std::error::Error + Send + Sync>>;
    
    /// 更新文档
    async fn update_document(
        &self, 
        index_name: &str, 
        doc_id: &str,
        fields: HashMap<String, String>
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    /// 删除文档
    async fn delete_document(&self, index_name: &str, doc_id: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    /// 全文搜索
    async fn search(
        &self, 
        index_name: &str, 
        query: &str,
        limit: Option<usize>
    ) -> Result<Vec<SearchResult>, Box<dyn std::error::Error + Send + Sync>>;
    
    /// 多字段搜索
    async fn search_multi_field(
        &self,
        index_name: &str,
        field_queries: HashMap<String, String>,
        limit: Option<usize>
    ) -> Result<Vec<SearchResult>, Box<dyn std::error::Error + Send + Sync>>;
    
    /// 获取索引统计信息
    async fn get_stats(&self) -> Result<IndexStats, Box<dyn std::error::Error + Send + Sync>>;
    
    /// 关闭索引服务
    async fn shutdown(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

/// 搜索结果
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub doc_id: String,
    pub score: f32,
    pub fields: HashMap<String, String>,
}

/// 索引统计信息
#[derive(Debug, Clone)]
pub struct IndexStats {
    pub total_indices: usize,
    pub index_names: Vec<String>,
}

/// 全文索引服务实现
pub struct IndexServiceImpl {
    storage_engine: Arc<tokio::sync::RwLock<TantivyStorageEngine>>,
    base_path: String,
    schema_name: String,
}

impl IndexServiceImpl {
    /// 创建新的 IndexService 实例
    pub async fn new(base_path: &str, schema_name: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let index_path = format!("{}/{}", base_path, schema_name);
        
        let storage_engine = TantivyStorageEngine::new(&index_path, schema_name)?;
        
        info!("IndexService initialized at path: {}", index_path);
        
        Ok(Self {
            storage_engine: Arc::new(tokio::sync::RwLock::new(storage_engine)),
            base_path: base_path.to_string(),
            schema_name: schema_name.to_string(),
        })
    }
    
    /// 获取存储引擎的引用
    pub fn storage_engine(&self) -> &Arc<tokio::sync::RwLock<TantivyStorageEngine>> {
        &self.storage_engine
    }
    
    /// 获取 schema 名称
    pub fn get_schema_name(&self) -> &str {
        &self.schema_name
    }
}

#[async_trait::async_trait]
impl IndexService for IndexServiceImpl {
    async fn init(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("Initializing IndexService for schema: {}", self.schema_name);
        // 初始化时可以加载已有索引或创建默认索引
        Ok(())
    }
    
    async fn create_index(
        &self, 
        index_name: &str, 
        fields: &[(u32, &str, ColumnType, Option<&str>)]
    ) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        info!("Creating index '{}' with {} fields", index_name, fields.len());
        
        let mut engine = self.storage_engine.write().await;
        let table_id = engine.create_table(index_name, Some("Full-text search index"), fields).await?;
        
        info!("Index '{}' created with ID: {}", index_name, table_id);
        Ok(table_id)
    }
    
    async fn drop_index(&self, index_name: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("Dropping index '{}'", index_name);
        
        let mut engine = self.storage_engine.write().await;
        engine.drop_table(index_name).await?;
        
        info!("Index '{}' dropped successfully", index_name);
        Ok(())
    }
    
    async fn list_indices(&self) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        debug!("Listing all indices");
        
        let engine = self.storage_engine.read().await;
        let tables = engine.list_tables().await?;
        
        debug!("Found {} indices", tables.len());
        Ok(tables)
    }
    
    async fn get_index_fields(&self, index_name: &str) -> Result<Vec<ColumnMeta>, Box<dyn std::error::Error + Send + Sync>> {
        debug!("Getting fields for index '{}'", index_name);
        
        let engine = self.storage_engine.read().await;
        let columns = engine.list_table_cols(index_name).await?;
        
        Ok(columns)
    }
    
    async fn get_index_meta(&self, index_name: &str) -> Result<Option<TableMeta>, Box<dyn std::error::Error + Send + Sync>> {
        debug!("Getting metadata for index '{}'", index_name);
        
        let engine = self.storage_engine.read().await;
        let meta = engine.get_table_meta(index_name).await?;
        
        Ok(meta)
    }
    
    async fn add_document(
        &self, 
        index_name: &str, 
        doc_id: &str,
        fields: HashMap<String, String>
    ) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        debug!("Adding document '{}' to index '{}'", doc_id, index_name);
        
        // TODO: 实现文档添加逻辑
        // 需要将 HashMap<String, String> 转换为 Row 结构
        warn!("add_document not fully implemented yet");
        
        Ok(0)
    }
    
    async fn update_document(
        &self, 
        index_name: &str, 
        doc_id: &str,
        fields: HashMap<String, String>
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        debug!("Updating document '{}' in index '{}'", doc_id, index_name);
        
        // TODO: 实现文档更新逻辑
        warn!("update_document not fully implemented yet");
        
        Ok(())
    }
    
    async fn delete_document(&self, index_name: &str, doc_id: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        debug!("Deleting document '{}' from index '{}'", doc_id, index_name);
        
        // TODO: 实现文档删除逻辑
        warn!("delete_document not fully implemented yet");
        
        Ok(())
    }
    
    async fn search(
        &self, 
        index_name: &str, 
        query: &str,
        limit: Option<usize>
    ) -> Result<Vec<SearchResult>, Box<dyn std::error::Error + Send + Sync>> {
        info!("Searching index '{}' with query: '{}'", index_name, query);
        
        // TODO: 实现全文搜索逻辑
        // 需要使用 Tantivy 的查询功能
        warn!("search not fully implemented yet");
        
        Ok(vec![])
    }
    
    async fn search_multi_field(
        &self,
        index_name: &str,
        field_queries: HashMap<String, String>,
        limit: Option<usize>
    ) -> Result<Vec<SearchResult>, Box<dyn std::error::Error + Send + Sync>> {
        info!("Multi-field search in index '{}' with {} field queries", index_name, field_queries.len());
        
        // TODO: 实现多字段搜索逻辑
        warn!("search_multi_field not fully implemented yet");
        
        Ok(vec![])
    }
    
    async fn get_stats(&self) -> Result<IndexStats, Box<dyn std::error::Error + Send + Sync>> {
        debug!("Getting index statistics");
        
        let engine = self.storage_engine.read().await;
        let indices = engine.list_tables().await?;
        
        Ok(IndexStats {
            total_indices: indices.len(),
            index_names: indices,
        })
    }
    
    async fn shutdown(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("Shutting down IndexService for schema: {}", self.schema_name);
        
        let mut engine = self.storage_engine.write().await;
        engine.shutdown().await?;
        
        info!("IndexService shutdown complete");
        Ok(())
    }
}

impl Clone for IndexServiceImpl {
    fn clone(&self) -> Self {
        Self {
            storage_engine: Arc::clone(&self.storage_engine),
            base_path: self.base_path.clone(),
            schema_name: self.schema_name.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn setup_test_service() -> (IndexServiceImpl, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let base_path = temp_dir.path().to_str().unwrap().to_string();
        let service = IndexServiceImpl::new(&base_path, "test_index").await.unwrap();
        (service, temp_dir)
    }

    #[tokio::test]
    async fn test_create_index_service() {
        let temp_dir = TempDir::new().unwrap();
        let base_path = temp_dir.path().to_str().unwrap();
        
        let result = IndexServiceImpl::new(base_path, "test").await;
        assert!(result.is_ok());
        
        let service = result.unwrap();
        assert_eq!(service.get_schema_name(), "test");
    }

    #[tokio::test]
    async fn test_create_index() {
        let (service, _temp_dir) = setup_test_service().await;
        
        let fields = vec![
            (0u32, "title", ColumnType::COLUMN_TYPE_STRING, Some("Title field")),
            (1u32, "content", ColumnType::COLUMN_TYPE_STRING, Some("Content field")),
        ];
        
        let result = service.create_index("test_index", &fields).await;
        assert!(result.is_ok());
        
        let index_id = result.unwrap();
        assert!(index_id > 0);
    }

    #[tokio::test]
    async fn test_list_indices() {
        let (service, _temp_dir) = setup_test_service().await;
        
        // 初始状态应该为空
        let indices = service.list_indices().await.unwrap();
        assert!(indices.is_empty());
        
        // 创建索引
        let fields = vec![(0u32, "field1", ColumnType::COLUMN_TYPE_STRING, None)];
        service.create_index("index1", &fields).await.unwrap();
        service.create_index("index2", &fields).await.unwrap();
        
        // 验证列表
        let indices = service.list_indices().await.unwrap();
        assert_eq!(indices.len(), 2);
        assert!(indices.contains(&"index1".to_string()));
        assert!(indices.contains(&"index2".to_string()));
    }

    #[tokio::test]
    async fn test_drop_index() {
        let (service, _temp_dir) = setup_test_service().await;
        
        let fields = vec![(0u32, "field1", ColumnType::COLUMN_TYPE_STRING, None)];
        service.create_index("test_index", &fields).await.unwrap();
        
        // 验证索引存在
        let indices = service.list_indices().await.unwrap();
        assert_eq!(indices.len(), 1);
        
        // 删除索引
        let result = service.drop_index("test_index").await;
        assert!(result.is_ok());
        
        // 验证索引已删除
        let indices = service.list_indices().await.unwrap();
        assert!(indices.is_empty());
    }

    #[tokio::test]
    async fn test_get_index_fields() {
        let (service, _temp_dir) = setup_test_service().await;
        
        let fields = vec![
            (0u32, "title", ColumnType::COLUMN_TYPE_STRING, None),
            (1u32, "content", ColumnType::COLUMN_TYPE_STRING, None),
        ];
        service.create_index("test_index", &fields).await.unwrap();
        
        let cols = service.get_index_fields("test_index").await.unwrap();
        assert_eq!(cols.len(), 2);
    }

    #[tokio::test]
    async fn test_get_stats() {
        let (service, _temp_dir) = setup_test_service().await;
        
        // 初始状态
        let stats = service.get_stats().await.unwrap();
        assert_eq!(stats.total_indices, 0);
        assert!(stats.index_names.is_empty());
        
        // 创建索引后
        let fields = vec![(0u32, "field1", ColumnType::COLUMN_TYPE_STRING, None)];
        service.create_index("index1", &fields).await.unwrap();
        
        let stats = service.get_stats().await.unwrap();
        assert_eq!(stats.total_indices, 1);
        assert!(stats.index_names.contains(&"index1".to_string()));
    }
}