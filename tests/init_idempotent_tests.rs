use laoflchDB_rust::{DatabaseService, DatabaseServiceImpl};
use laoflchdb_engines::{Row, RowType, SpecialFields, Message, field::field::Value, field::{String, Integer}};
use protobuf::CodedOutputStream;
use std::sync::Arc;

async fn create_test_service_with_path(db_path: &str) -> Arc<dyn DatabaseService> {
    let service = DatabaseServiceImpl::new(db_path).await;
    Arc::new(service)
}

#[tokio::test]
async fn test_init_database_idempotent() {
    let temp_dir = std::env::temp_dir();
    let db_path = temp_dir.join(format!("test_init_idempotent_{}", uuid::Uuid::new_v4()));
    let db_path_str = db_path.to_str().unwrap();
    
    let service = create_test_service_with_path(db_path_str).await;
    
    let result = service.init_database().await;
    assert!(result.is_ok(), "第一次初始化失败");
    
    let schemas = service.list_schemas().await.unwrap();
    assert!(schemas.contains(&"sys".to_string()), "sys Schema 应该存在");
    
    let tables = service.list_tables("sys").await.unwrap();
    assert!(tables.contains(&"user".to_string()), "user 表应该存在");
    
    let result = service.init_database().await;
    assert!(result.is_ok(), "第二次初始化应该成功（幂等）");
    
    let schemas = service.list_schemas().await.unwrap();
    assert!(schemas.contains(&"sys".to_string()), "sys Schema 应该仍然存在");
    
    let tables = service.list_tables("sys").await.unwrap();
    assert!(tables.contains(&"user".to_string()), "user 表应该仍然存在");
}

#[tokio::test]
async fn test_init_database_preserves_data() {
    let temp_dir = std::env::temp_dir();
    let db_path = temp_dir.join(format!("test_init_preserve_data_{}", uuid::Uuid::new_v4()));
    let db_path_str = db_path.to_str().unwrap();
    
    let service = create_test_service_with_path(db_path_str).await;
    
    service.init_database().await.unwrap();
    
    let columns = vec![
        (0u32, "id", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64),
        (1u32, "name", laoflchdb_engines::ColumnType::COLUMN_TYPE_STRING),
    ];
    
    service.create_table("sys", "test_data", &columns).await.unwrap();
    
    let mut row = Row::new();
    row.row_type = RowType::ROW_TYPE_NORMAL.into();
    row.version = 1;
    
    let mut field1 = laoflchdb_engines::Field::new();
    field1.value = Some(Value::IntegerValue(Integer { value: 1, special_fields: SpecialFields::default() }));
    let mut buf1 = Vec::new();
    {
        let mut os = CodedOutputStream::vec(&mut buf1);
        field1.write_to(&mut os).unwrap();
        os.flush().unwrap();
    }
    row.data.push(buf1);
    
    let mut field2 = laoflchdb_engines::Field::new();
    field2.value = Some(Value::StringValue(String { value: "test_value".to_string(), special_fields: SpecialFields::default() }));
    let mut buf2 = Vec::new();
    {
        let mut os = CodedOutputStream::vec(&mut buf2);
        field2.write_to(&mut os).unwrap();
        os.flush().unwrap();
    }
    row.data.push(buf2);
    
    service.add_row("sys", "test_data", &row).await.unwrap();
    
    let tables_before = service.list_tables("sys").await.unwrap();
    assert!(tables_before.contains(&"test_data".to_string()), "test_data 表应该存在");
    
    service.init_database().await.unwrap();
    
    let tables_after = service.list_tables("sys").await.unwrap();
    assert!(tables_after.contains(&"test_data".to_string()), "test_data 表在第二次初始化后应该仍然存在");
}


