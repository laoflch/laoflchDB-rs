use laoflchdb_index_tantivy_engine::TantivyStorageEngine;
use laoflchdb_engines::{ColumnType, StorageEngine};
use tempfile::TempDir;

fn setup_test_engine() -> (TantivyStorageEngine, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let index_path = temp_dir.path().join("test_index").to_str().unwrap().to_string();
    let engine = TantivyStorageEngine::new(&index_path, "test_schema").unwrap();
    (engine, temp_dir)
}

#[tokio::test]
async fn test_create_engine() {
    let temp_dir = TempDir::new().unwrap();
    let index_path = temp_dir.path().join("test_index");

    let result = TantivyStorageEngine::new(index_path.to_str().unwrap(), "test_schema");
    assert!(result.is_ok());

    let engine = result.unwrap();
    assert_eq!(engine.get_schema_name(), "test_schema");

    // 验证目录已创建
    assert!(index_path.exists());
}

#[tokio::test]
async fn test_create_table() {
    let (mut engine, _temp_dir) = setup_test_engine();

    let columns = vec![
        (0u32, "title", ColumnType::COLUMN_TYPE_STRING, Some("Title field")),
        (1u32, "content", ColumnType::COLUMN_TYPE_STRING, Some("Content field")),
        (2u32, "count", ColumnType::COLUMN_TYPE_INT64, Some("Count field")),
    ];

    let result = engine.create_table("test_table", Some("Test table"), &columns).await;
    assert!(result.is_ok());

    let table_id = result.unwrap();
    assert!(table_id > 0);
}

#[tokio::test]
async fn test_create_multiple_tables() {
    let (mut engine, _temp_dir) = setup_test_engine();

    let columns1 = vec![
        (0u32, "name", ColumnType::COLUMN_TYPE_STRING, None),
        (1u32, "value", ColumnType::COLUMN_TYPE_FLOAT, None),
    ];

    let columns2 = vec![
        (0u32, "id", ColumnType::COLUMN_TYPE_INT64, None),
        (1u32, "description", ColumnType::COLUMN_TYPE_STRING, None),
    ];

    let result1 = engine.create_table("table1", None, &columns1).await;
    let result2 = engine.create_table("table2", None, &columns2).await;

    assert!(result1.is_ok());
    assert!(result2.is_ok());
}

#[tokio::test]
async fn test_list_tables() {
    let (mut engine, _temp_dir) = setup_test_engine();

    // 初始状态应该为空
    let tables = engine.list_tables().await.unwrap();
    assert!(tables.is_empty());

    // 创建表
    let columns = vec![(0u32, "field1", ColumnType::COLUMN_TYPE_STRING, None)];
    engine.create_table("table1", None, &columns).await.unwrap();
    engine.create_table("table2", None, &columns).await.unwrap();

    // 验证列表
    let tables = engine.list_tables().await.unwrap();
    assert_eq!(tables.len(), 2);
    assert!(tables.contains(&"table1".to_string()));
    assert!(tables.contains(&"table2".to_string()));
}

#[tokio::test]
async fn test_drop_table() {
    let (mut engine, _temp_dir) = setup_test_engine();

    let columns = vec![(0u32, "field1", ColumnType::COLUMN_TYPE_STRING, None)];
    engine.create_table("test_table", None, &columns).await.unwrap();

    // 验证表存在
    let tables = engine.list_tables().await.unwrap();
    assert_eq!(tables.len(), 1);

    // 删除表
    let result = engine.drop_table("test_table").await;
    assert!(result.is_ok());

    // 验证表已删除
    let tables = engine.list_tables().await.unwrap();
    assert!(tables.is_empty());
}

#[tokio::test]
async fn test_list_table_cols() {
    let (mut engine, _temp_dir) = setup_test_engine();

    let columns = vec![
        (0u32, "title", ColumnType::COLUMN_TYPE_STRING, None),
        (1u32, "count", ColumnType::COLUMN_TYPE_INT64, None),
        (2u32, "score", ColumnType::COLUMN_TYPE_FLOAT, None),
    ];

    engine.create_table("test_table", None, &columns).await.unwrap();

    let cols = engine.list_table_cols("test_table").await.unwrap();
    assert_eq!(cols.len(), 3);
}

#[tokio::test]
async fn test_list_table_cols_not_found() {
    let (engine, _temp_dir) = setup_test_engine();

    let result = engine.list_table_cols("non_existent_table").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_get_table_meta() {
    let (mut engine, _temp_dir) = setup_test_engine();

    let columns = vec![(0u32, "field1", ColumnType::COLUMN_TYPE_STRING, None)];
    engine.create_table("test_table", Some("Test comment"), &columns).await.unwrap();

    let meta = engine.get_table_meta("test_table").await.unwrap();
    assert!(meta.is_some());

    let table_meta = meta.unwrap();
    assert_eq!(table_meta.table_name, "test_table");
    assert_eq!(table_meta.comment, "Test comment");
    assert_eq!(table_meta.column_count, 1);
}

#[tokio::test]
async fn test_get_table_meta_not_found() {
    let (engine, _temp_dir) = setup_test_engine();

    let meta = engine.get_table_meta("non_existent_table").await.unwrap();
    assert!(meta.is_none());
}

#[tokio::test]
async fn test_update_table_comment() {
    let (mut engine, _temp_dir) = setup_test_engine();

    let columns = vec![(0u32, "field1", ColumnType::COLUMN_TYPE_STRING, None)];
    engine.create_table("test_table", Some("Original comment"), &columns).await.unwrap();

    let result = engine.update_table_comment("test_table", "Updated comment").await;
    assert!(result.is_ok());

    let meta = engine.get_table_meta("test_table").await.unwrap().unwrap();
    assert_eq!(meta.comment, "Updated comment");
}

#[tokio::test]
async fn test_update_table_comment_not_found() {
    let (mut engine, _temp_dir) = setup_test_engine();

    let result = engine.update_table_comment("non_existent_table", "New comment").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_get_schema_info() {
    let (engine, _temp_dir) = setup_test_engine();

    let info = engine.get_schema_info().await.unwrap();
    assert!(info.contains("test_schema"));
    assert!(info.contains("test_index"));
}

#[tokio::test]
async fn test_get_all_meta() {
    let (mut engine, _temp_dir) = setup_test_engine();

    // 初始状态
    let meta = engine.get_all_meta().await.unwrap();
    assert!(meta.contains("test_schema"));
    assert!(meta.contains("Tables count: 0"));

    // 创建表后
    let columns = vec![(0u32, "field1", ColumnType::COLUMN_TYPE_STRING, None)];
    engine.create_table("table1", None, &columns).await.unwrap();

    let meta = engine.get_all_meta().await.unwrap();
    assert!(meta.contains("Tables count: 1"));
    assert!(meta.contains("table1"));
}

#[tokio::test]
async fn test_get_column_types() {
    let (mut engine, _temp_dir) = setup_test_engine();

    let columns = vec![
        (0u32, "title", ColumnType::COLUMN_TYPE_STRING, None),
        (1u32, "count", ColumnType::COLUMN_TYPE_INT64, None),
    ];

    engine.create_table("test_table", None, &columns).await.unwrap();

    let col_types = engine.get_column_types("test_table").await.unwrap();
    assert_eq!(col_types.len(), 2);
    assert!(col_types.contains_key("title"));
    assert!(col_types.contains_key("count"));
}

#[tokio::test]
async fn test_get_column_types_not_found() {
    let (engine, _temp_dir) = setup_test_engine();

    let result = engine.get_column_types("non_existent_table").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_drop_non_existent_table() {
    let (mut engine, _temp_dir) = setup_test_engine();

    // 删除不存在的表应该不会报错
    let result = engine.drop_table("non_existent_table").await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_concurrent_table_creation() {
    let (mut engine, _temp_dir) = setup_test_engine();

    let columns = vec![(0u32, "field1", ColumnType::COLUMN_TYPE_STRING, None)];

    // 创建多个表
    for i in 0..10 {
        let table_name = format!("table_{}", i);
        let result = engine.create_table(&table_name, None, &columns).await;
        assert!(result.is_ok());
    }

    let tables = engine.list_tables().await.unwrap();
    assert_eq!(tables.len(), 10);
}
