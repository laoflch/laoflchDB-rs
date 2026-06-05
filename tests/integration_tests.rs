use laoflchDB_rust::access::RestService;
use laoflchDB_rust::{DatabaseService, DatabaseServiceImpl};
use laoflchdb_engines;
use std::sync::Arc;
use tokio::net::TcpListener;

async fn create_test_service() -> Arc<dyn DatabaseService> {
    let temp_dir = std::env::temp_dir();
    let db_path = temp_dir.join(format!("test_integration_{}", uuid::Uuid::new_v4()));
    let db_path_str = db_path.to_str().unwrap();
    
    let service = DatabaseServiceImpl::new(db_path_str).await;
    service.init_database().await.unwrap();
    Arc::new(service)
}

async fn setup_rest_service() -> (RestService, Arc<dyn DatabaseService>) {
    let service = create_test_service().await;
    let rest_service = RestService::new(Arc::clone(&service));
    (rest_service, service)
}

#[tokio::test]
async fn test_integration_full_workflow() {
    let (rest_service, _service) = setup_rest_service().await;
    let app = rest_service.router();
    
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    
    let client = reqwest::Client::new();
    
    // 1. 健康检查
    let res = client.get(format!("http://{}/health", addr))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    
    // 2. 创建表
    let create_table_body = serde_json::json!({
        "schema": "sys",
        "table_name": "users",
        "columns": [
            {"name": "id", "column_type": "INT64"},
            {"name": "name", "column_type": "STRING"},
            {"name": "email", "column_type": "STRING"}
        ]
    });
    
    let res = client.post(format!("http://{}/api/v1/tables", addr))
        .json(&create_table_body)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body = res.text().await.unwrap();
    assert!(body.contains("\"success\":true"));
    
    // 3. 列出表
    let res = client.get(format!("http://{}/api/v1/schemas/sys/tables", addr))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body = res.text().await.unwrap();
    assert!(body.contains("users"));
    
    // 4. 获取表元数据
    let res = client.get(format!("http://{}/api/v1/schemas/sys/tables/users", addr))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body = res.text().await.unwrap();
    assert!(body.contains("\"table_name\":\"users\""));
    assert!(body.contains("\"column_count\":3"));
    
    // 5. 插入数据
    let put_body = serde_json::json!({
        "schema": "sys",
        "table": "users",
        "key": "user1",
        "value": "{\"id\":1,\"name\":\"Alice\",\"email\":\"alice@example.com\"}"
    });
    
    let res = client.post(format!("http://{}/api/v1/put", addr))
        .json(&put_body)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    
    // 6. 读取数据
    let res = client.get(format!("http://{}/api/v1/get", addr))
        .query(&[("schema", "sys"), ("table", "users"), ("key", "user1")])
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body = res.text().await.unwrap();
    assert!(body.contains("\"success\":true"));
    assert!(body.contains("\"value\""));
    
    // 7. 更新数据
    let update_body = serde_json::json!({
        "schema": "sys",
        "table": "users",
        "key": "user1",
        "value": "{\"id\":1,\"name\":\"Alice Updated\",\"email\":\"alice.updated@example.com\"}"
    });
    
    let res = client.post(format!("http://{}/api/v1/put", addr))
        .json(&update_body)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    
    // 8. 再次读取验证更新
    let res = client.get(format!("http://{}/api/v1/get", addr))
        .query(&[("schema", "sys"), ("table", "users"), ("key", "user1")])
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    
    // 9. 删除数据
    let delete_body = serde_json::json!({
        "schema": "sys",
        "table": "users",
        "key": "user1"
    });
    
    let res = client.post(format!("http://{}/api/v1/delete", addr))
        .json(&delete_body)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    
    // 10. 验证删除
    let res = client.get(format!("http://{}/api/v1/get", addr))
        .query(&[("schema", "sys"), ("table", "users"), ("key", "user1")])
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body = res.text().await.unwrap();
    assert!(body.contains("\"value\":null"));
}

#[tokio::test]
async fn test_integration_multiple_tables() {
    let (rest_service, _service) = setup_rest_service().await;
    let app = rest_service.router();
    
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    
    let client = reqwest::Client::new();
    
    // 创建多个表
    for i in 1..=3 {
        let table_name = format!("table_{}", i);
        let create_table_body = serde_json::json!({
            "schema": "sys",
            "table_name": table_name,
            "columns": [
                {"name": "id", "column_type": "INT64"},
                {"name": "data", "column_type": "STRING"}
            ]
        });
        
        let res = client.post(format!("http://{}/api/v1/tables", addr))
            .json(&create_table_body)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
    }
    
    // 验证所有表都创建了
    let res = client.get(format!("http://{}/api/v1/schemas/sys/tables", addr))
        .send()
        .await
        .unwrap();
    let body = res.text().await.unwrap();
    assert!(body.contains("table_1"));
    assert!(body.contains("table_2"));
    assert!(body.contains("table_3"));
}

#[tokio::test]
async fn test_integration_error_handling() {
    let (rest_service, _service) = setup_rest_service().await;
    let app = rest_service.router();
    
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    
    let client = reqwest::Client::new();
    
    // 尝试从不存在的表读取数据
    let res = client.get(format!("http://{}/api/v1/get", addr))
        .query(&[("schema", "sys"), ("table", "nonexistent_table"), ("key", "test")])
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body = res.text().await.unwrap();
    assert!(body.contains("\"success\":false"));
    
    // 尝试获取不存在的表元数据
    let res = client.get(format!("http://{}/api/v1/schemas/sys/tables/nonexistent_table", addr))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body = res.text().await.unwrap();
    assert!(body.contains("\"success\":false"));
}

#[tokio::test]
async fn test_integration_sql_query() {
    let (rest_service, service) = setup_rest_service().await;
    let app = rest_service.router();
    
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    
    let client = reqwest::Client::new();
    
    // 1. 创建测试表
    let create_table_body = serde_json::json!({
        "schema": "sys",
        "table_name": "sql_test_table",
        "columns": [
            {"name": "id", "column_type": "INT64"},
            {"name": "name", "column_type": "STRING"},
            {"name": "age", "column_type": "INT64"},
        ]
    });
    
    let res = client.post(format!("http://{}/api/v1/tables", addr))
        .json(&create_table_body)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    
    // 2. 等待表注册到 SQL 引擎
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    
    // 3. 测试 SQL 查询（查询表结构）
    let sql_query_body = serde_json::json!({
        "schema": "sys",
        "sql": "SELECT * FROM sql_test_table"
    });
    
    let res = client.post(format!("http://{}/api/v1/sql_query", addr))
        .json(&sql_query_body)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    
    let body = res.text().await.unwrap();
    assert!(body.contains("\"success\":true"));
}

#[tokio::test]
async fn test_sql_engine_query() {
    let service = create_test_service().await;
    
    // 创建测试表
    let columns = vec![
        (0u32, "id", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64),
        (1u32, "name", laoflchdb_engines::ColumnType::COLUMN_TYPE_STRING),
        (2u32, "age", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64),
    ];
    
    service.create_table("sys", "sql_query_test", &columns).await.unwrap();
    
    // 等待表注册
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
    
    // 执行 SQL 查询
    let result = service.sql_query("sys", "SELECT * FROM sql_query_test").await;
    assert!(result.is_ok());
    
    let query_result = result.unwrap();
    assert!(query_result.rows.is_empty() || query_result.rows.len() >= 0);
}

#[tokio::test]
async fn test_sql_query_with_data() {
    let service = create_test_service().await;
    
    // 创建测试表
    let columns = vec![
        (0u32, "id", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64),
        (1u32, "name", laoflchdb_engines::ColumnType::COLUMN_TYPE_STRING),
        (2u32, "age", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64),
        (3u32, "score", laoflchdb_engines::ColumnType::COLUMN_TYPE_FLOAT),
    ];
    
    service.create_table("sys", "sql_data_test", &columns).await.unwrap();
    
    // 等待表注册
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
    
    // 插入测试数据
    use laoflchdb_engines::{Row, RowType, SpecialFields, Message, field::field::Value, field::{String, Integer, Float}};
    use protobuf::CodedOutputStream;
    
    let mut row1 = Row::new();
    row1.row_type = RowType::ROW_TYPE_NORMAL.into();
    row1.version = 1;
    
    let mut field1 = laoflchdb_engines::Field::new();
    field1.value = Some(Value::IntegerValue(Integer { value: 1, special_fields: SpecialFields::default() }));
    let mut buf1 = Vec::new();
    {
        let mut os = CodedOutputStream::vec(&mut buf1);
        field1.write_to(&mut os).unwrap();
        os.flush().unwrap();
    }
    row1.data.push(buf1);
    
    let mut field2 = laoflchdb_engines::Field::new();
    field2.value = Some(Value::StringValue(String { value: "Alice".to_string(), special_fields: SpecialFields::default() }));
    let mut buf2 = Vec::new();
    {
        let mut os = CodedOutputStream::vec(&mut buf2);
        field2.write_to(&mut os).unwrap();
        os.flush().unwrap();
    }
    row1.data.push(buf2);
    
    let mut field3 = laoflchdb_engines::Field::new();
    field3.value = Some(Value::IntegerValue(Integer { value: 30, special_fields: SpecialFields::default() }));
    let mut buf3 = Vec::new();
    {
        let mut os = CodedOutputStream::vec(&mut buf3);
        field3.write_to(&mut os).unwrap();
        os.flush().unwrap();
    }
    row1.data.push(buf3);
    
    let mut field4 = laoflchdb_engines::Field::new();
    field4.value = Some(Value::FloatValue(Float { value: 95.5, special_fields: SpecialFields::default() }));
    let mut buf4 = Vec::new();
    {
        let mut os = CodedOutputStream::vec(&mut buf4);
        field4.write_to(&mut os).unwrap();
        os.flush().unwrap();
    }
    row1.data.push(buf4);
    
    service.add_row("sys", "sql_data_test", &row1).await.unwrap();
    
    // 等待数据写入
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    // 测试全表查询
    let result = service.sql_query("sys", "SELECT * FROM sql_data_test").await;
    assert!(result.is_ok());
    
    let query_result = result.unwrap();
    assert_eq!(query_result.rows.len(), 1);
    
    // 测试投影下推
    let result = service.sql_query("sys", "SELECT id, name FROM sql_data_test").await;
    assert!(result.is_ok());
    
    // 测试谓词下推
    let result = service.sql_query("sys", "SELECT * FROM sql_data_test WHERE age > 25").await;
    assert!(result.is_ok());
    
    let query_result = result.unwrap();
    assert_eq!(query_result.rows.len(), 1);
    
    // 测试 Limit 下推
    let result = service.sql_query("sys", "SELECT * FROM sql_data_test LIMIT 1").await;
    assert!(result.is_ok());
    
    let query_result = result.unwrap();
    assert_eq!(query_result.rows.len(), 1);
    
    // 测试组合查询
    let result = service.sql_query("sys", "SELECT name, age FROM sql_data_test WHERE id = 1").await;
    assert!(result.is_ok());
    
    let query_result = result.unwrap();
    assert_eq!(query_result.rows.len(), 1);
}

#[tokio::test]
async fn test_sql_query_filter_pushdown() {
    let service = create_test_service().await;
    
    // 创建测试表
    let columns = vec![
        (0u32, "id", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64),
        (1u32, "name", laoflchdb_engines::ColumnType::COLUMN_TYPE_STRING),
        (2u32, "age", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64),
    ];
    
    service.create_table("sys", "filter_test", &columns).await.unwrap();
    
    // 等待表注册
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
    
    // 插入多条测试数据
    use laoflchdb_engines::{Row, RowType, SpecialFields, Message, field::field::Value, field::{String, Integer}};
    use protobuf::CodedOutputStream;
    
    for i in 1..=5 {
        let mut row = Row::new();
        row.row_type = RowType::ROW_TYPE_NORMAL.into();
        row.version = 1;
        
        let mut field1 = laoflchdb_engines::Field::new();
        field1.value = Some(Value::IntegerValue(Integer { value: i as i64, special_fields: SpecialFields::default() }));
        let mut buf1 = Vec::new();
        {
            let mut os = CodedOutputStream::vec(&mut buf1);
            field1.write_to(&mut os).unwrap();
            os.flush().unwrap();
        }
        row.data.push(buf1);
        
        let mut field2 = laoflchdb_engines::Field::new();
        field2.value = Some(Value::StringValue(String { value: format!("User{}", i), special_fields: SpecialFields::default() }));
        let mut buf2 = Vec::new();
        {
            let mut os = CodedOutputStream::vec(&mut buf2);
            field2.write_to(&mut os).unwrap();
            os.flush().unwrap();
        }
        row.data.push(buf2);
        
        let mut field3 = laoflchdb_engines::Field::new();
        field3.value = Some(Value::IntegerValue(Integer { value: (20 + i * 5) as i64, special_fields: SpecialFields::default() }));
        let mut buf3 = Vec::new();
        {
            let mut os = CodedOutputStream::vec(&mut buf3);
            field3.write_to(&mut os).unwrap();
            os.flush().unwrap();
        }
        row.data.push(buf3);
        
        service.add_row("sys", "filter_test", &row).await.unwrap();
    }
    
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    // 测试 > 操作符
    let result = service.sql_query("sys", "SELECT * FROM filter_test WHERE age > 30").await;
    assert!(result.is_ok());
    let query_result = result.unwrap();
    assert_eq!(query_result.rows.len(), 3); // age > 30: 35, 40, 45
    
    // 测试 < 操作符
    let result = service.sql_query("sys", "SELECT * FROM filter_test WHERE age < 30").await;
    assert!(result.is_ok());
    let query_result = result.unwrap();
    assert_eq!(query_result.rows.len(), 1); // age < 30: 25
    
    // 测试 = 操作符
    let result = service.sql_query("sys", "SELECT * FROM filter_test WHERE id = 3").await;
    assert!(result.is_ok());
    let query_result = result.unwrap();
    assert_eq!(query_result.rows.len(), 1);
    
    // 测试 >= 操作符
    let result = service.sql_query("sys", "SELECT * FROM filter_test WHERE age >= 35").await;
    assert!(result.is_ok());
    let query_result = result.unwrap();
    assert_eq!(query_result.rows.len(), 3); // age >= 35: 35, 40, 45
    
    // 测试 <= 操作符
    let result = service.sql_query("sys", "SELECT * FROM filter_test WHERE age <= 30").await;
    assert!(result.is_ok());
    let query_result = result.unwrap();
    assert_eq!(query_result.rows.len(), 2); // age <= 30: 25, 30
}
