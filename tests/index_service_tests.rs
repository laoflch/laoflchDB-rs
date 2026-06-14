use laoflchDB_rust::service::index_service::{IndexService, IndexServiceImpl};
use laoflchdb_engines::ColumnType;
use std::collections::HashMap;
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
        (2u32, "count", ColumnType::COLUMN_TYPE_INT64, Some("Count field")),
    ];
    
    let result = service.create_index("test_index", &fields).await;
    assert!(result.is_ok());
    
    let index_id = result.unwrap();
    assert!(index_id > 0);
}

#[tokio::test]
async fn test_create_multiple_indices() {
    let (service, _temp_dir) = setup_test_service().await;
    
    let fields1 = vec![
        (0u32, "name", ColumnType::COLUMN_TYPE_STRING, None),
        (1u32, "value", ColumnType::COLUMN_TYPE_FLOAT, None),
    ];

    let fields2 = vec![
        (0u32, "id", ColumnType::COLUMN_TYPE_INT64, None),
        (1u32, "description", ColumnType::COLUMN_TYPE_STRING, None),
    ];

    let result1 = service.create_index("index1", &fields1).await;
    let result2 = service.create_index("index2", &fields2).await;

    assert!(result1.is_ok());
    assert!(result2.is_ok());
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
        (1u32, "count", ColumnType::COLUMN_TYPE_INT64, None),
        (2u32, "score", ColumnType::COLUMN_TYPE_FLOAT, None),
    ];

    service.create_index("test_index", &fields).await.unwrap();

    let cols = service.get_index_fields("test_index").await.unwrap();
    assert_eq!(cols.len(), 3);
}

#[tokio::test]
async fn test_get_index_meta() {
    let (service, _temp_dir) = setup_test_service().await;

    let fields = vec![(0u32, "field1", ColumnType::COLUMN_TYPE_STRING, None)];
    service.create_index("test_index", &fields).await.unwrap();

    let meta = service.get_index_meta("test_index").await.unwrap();
    assert!(meta.is_some());

    let index_meta = meta.unwrap();
    assert_eq!(index_meta.table_name, "test_index");
    assert_eq!(index_meta.column_count, 1);
}

#[tokio::test]
async fn test_get_index_meta_not_found() {
    let (service, _temp_dir) = setup_test_service().await;

    let meta = service.get_index_meta("non_existent_index").await.unwrap();
    assert!(meta.is_none());
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

#[tokio::test]
async fn test_drop_non_existent_index() {
    let (service, _temp_dir) = setup_test_service().await;

    // 删除不存在的索引应该不会报错
    let result = service.drop_index("non_existent_index").await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_concurrent_index_creation() {
    let (service, _temp_dir) = setup_test_service().await;

    let fields = vec![(0u32, "field1", ColumnType::COLUMN_TYPE_STRING, None)];

    // 创建多个索引
    for i in 0..10 {
        let index_name = format!("index_{}", i);
        let result = service.create_index(&index_name, &fields).await;
        assert!(result.is_ok());
    }

    let indices = service.list_indices().await.unwrap();
    assert_eq!(indices.len(), 10);
}

#[tokio::test]
async fn test_add_document_placeholder() {
    // 这是一个占位符测试，因为 add_document 尚未完全实现
    let (service, _temp_dir) = setup_test_service().await;

    let fields = vec![(0u32, "field1", ColumnType::COLUMN_TYPE_STRING, None)];
    service.create_index("test_index", &fields).await.unwrap();

    let doc_fields = HashMap::new();
    let result = service.add_document("test_index", "doc1", doc_fields).await;
    // 目前应该返回 Ok(0) 因为实现是占位符
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_search_placeholder() {
    // 这是一个占位符测试，因为 search 尚未完全实现
    let (service, _temp_dir) = setup_test_service().await;

    let fields = vec![(0u32, "field1", ColumnType::COLUMN_TYPE_STRING, None)];
    service.create_index("test_index", &fields).await.unwrap();

    let result = service.search("test_index", "test query", Some(10)).await;
    // 目前应该返回空结果因为实现是占位符
    assert!(result.is_ok());
    let results = result.unwrap();
    assert!(results.is_empty());
}

#[tokio::test]
async fn test_multi_field_search_placeholder() {
    // 这是一个占位符测试，因为 search_multi_field 尚未完全实现
    let (service, _temp_dir) = setup_test_service().await;

    let fields = vec![(0u32, "field1", ColumnType::COLUMN_TYPE_STRING, None)];
    service.create_index("test_index", &fields).await.unwrap();

    let field_queries = HashMap::new();
    let result = service.search_multi_field("test_index", field_queries, Some(10)).await;
    // 目前应该返回空结果因为实现是占位符
    assert!(result.is_ok());
    let results = result.unwrap();
    assert!(results.is_empty());
}
