//! SQL 高级功能测试
//! 
//! 测试聚合函数、排序、分组等高级 SQL 功能

use laoflchDB_rust::{DatabaseService, DatabaseServiceImpl};
use laoflchdb_engines::{Row, RowType, SpecialFields, Message, field::field::Value, field::{String, Integer, Float}};
use protobuf::CodedOutputStream;
use std::sync::Arc;

/// 创建测试服务
async fn create_test_service() -> Arc<dyn DatabaseService> {
    let temp_dir = std::env::temp_dir();
    let db_path = temp_dir.join(format!("test_advanced_sql_{}", uuid::Uuid::new_v4()));
    let db_path_str = db_path.to_str().unwrap();
    
    let service = DatabaseServiceImpl::new(db_path_str).await;
    service.init_database().await.unwrap();
    Arc::new(service)
}

/// 辅助函数：插入测试数据
async fn insert_test_data(
    service: &Arc<dyn DatabaseService>,
    table_name: &str,
    data: Vec<(i64, std::string::String, i64, f64)>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    for (id, name, age, score) in data {
        let mut row = Row::new();
        row.row_type = RowType::ROW_TYPE_NORMAL.into();
        row.version = 1;
        
        // id
        let mut f = laoflchdb_engines::Field::new();
        f.value = Some(Value::IntegerValue(Integer { value: id, special_fields: SpecialFields::default() }));
        let mut buf = Vec::new();
        { let mut os = CodedOutputStream::vec(&mut buf); f.write_to(&mut os).unwrap(); os.flush().unwrap(); }
        row.data.push(buf);
        
        // name
        let mut f = laoflchdb_engines::Field::new();
        f.value = Some(Value::StringValue(String { value: name, special_fields: SpecialFields::default() }));
        let mut buf = Vec::new();
        { let mut os = CodedOutputStream::vec(&mut buf); f.write_to(&mut os).unwrap(); os.flush().unwrap(); }
        row.data.push(buf);
        
        // age
        let mut f = laoflchdb_engines::Field::new();
        f.value = Some(Value::IntegerValue(Integer { value: age, special_fields: SpecialFields::default() }));
        let mut buf = Vec::new();
        { let mut os = CodedOutputStream::vec(&mut buf); f.write_to(&mut os).unwrap(); os.flush().unwrap(); }
        row.data.push(buf);
        
        // score
        let mut f = laoflchdb_engines::Field::new();
        f.value = Some(Value::FloatValue(Float { value: score, special_fields: SpecialFields::default() }));
        let mut buf = Vec::new();
        { let mut os = CodedOutputStream::vec(&mut buf); f.write_to(&mut os).unwrap(); os.flush().unwrap(); }
        row.data.push(buf);
        
        service.add_row("sys", table_name, &row).await.unwrap();
    }
    
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    Ok(())
}

/// 测试聚合函数：COUNT
#[tokio::test]
async fn test_aggregation_count() {
    let service = create_test_service().await;
    
    // 创建测试表
    let columns = vec![
        (0u32, "id", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, None),
        (1u32, "name", laoflchdb_engines::ColumnType::COLUMN_TYPE_STRING, None),
        (2u32, "age", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, None),
        (3u32, "score", laoflchdb_engines::ColumnType::COLUMN_TYPE_FLOAT, None),
    ];
    
    service.create_table("sys", "agg_test", None, &columns).await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
    
    // 插入测试数据
    let test_data = vec![
        (1, std::string::String::from("Alice"), 30, 95.5),
        (2, std::string::String::from("Bob"), 25, 88.0),
        (3, std::string::String::from("Charlie"), 35, 92.5),
        (4, std::string::String::from("David"), 28, 90.0),
        (5, std::string::String::from("Eve"), 40, 85.0),
    ];
    insert_test_data(&service, "agg_test", test_data).await.unwrap();
    
    // 测试 COUNT(id) - 使用 COUNT(column) 而非 COUNT(*)
    let result = service.sql_query("sys", "SELECT COUNT(id) FROM agg_test").await;
    assert!(result.is_ok(), "COUNT(id) 查询失败");
    let query_result = result.unwrap();
    assert_eq!(query_result.rows.len(), 1, "COUNT(id) 应返回 1 行");
    
    // 测试 COUNT(name)
    let result = service.sql_query("sys", "SELECT COUNT(name) FROM agg_test").await;
    assert!(result.is_ok(), "COUNT(name) 查询失败");
    
    // 测试带 WHERE 的 COUNT
    let result = service.sql_query("sys", "SELECT COUNT(id) FROM agg_test WHERE age > 30").await;
    assert!(result.is_ok(), "带 WHERE 的 COUNT 查询失败");
    
    println!("COUNT 聚合函数测试通过!");
}

/// 测试聚合函数：SUM, AVG, MIN, MAX
#[tokio::test]
async fn test_aggregation_math() {
    let service = create_test_service().await;
    
    // 创建测试表
    let columns = vec![
        (0u32, "id", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, None),
        (1u32, "name", laoflchdb_engines::ColumnType::COLUMN_TYPE_STRING, None),
        (2u32, "age", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, None),
        (3u32, "score", laoflchdb_engines::ColumnType::COLUMN_TYPE_FLOAT, None),
    ];
    
    service.create_table("sys", "math_agg_test", None, &columns).await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
    
    // 插入测试数据
    let test_data = vec![
        (1, std::string::String::from("Alice"), 30, 95.5),
        (2, std::string::String::from("Bob"), 25, 88.0),
        (3, std::string::String::from("Charlie"), 35, 92.5),
        (4, std::string::String::from("David"), 28, 90.0),
        (5, std::string::String::from("Eve"), 40, 85.0),
    ];
    insert_test_data(&service, "math_agg_test", test_data).await.unwrap();
    
    // 测试 SUM
    let result = service.sql_query("sys", "SELECT SUM(score) FROM math_agg_test").await;
    assert!(result.is_ok(), "SUM 查询失败");
    
    // 测试 AVG
    let result = service.sql_query("sys", "SELECT AVG(age) FROM math_agg_test").await;
    assert!(result.is_ok(), "AVG 查询失败");
    
    // 测试 MIN
    let result = service.sql_query("sys", "SELECT MIN(age) FROM math_agg_test").await;
    assert!(result.is_ok(), "MIN 查询失败");
    
    // 测试 MAX
    let result = service.sql_query("sys", "SELECT MAX(score) FROM math_agg_test").await;
    assert!(result.is_ok(), "MAX 查询失败");
    
    // 测试多个聚合函数
    let result = service.sql_query("sys", "SELECT COUNT(*), SUM(score), AVG(age) FROM math_agg_test").await;
    assert!(result.is_ok(), "多个聚合函数查询失败");
    
    println!("数学聚合函数测试通过!");
}

/// 测试 GROUP BY
#[tokio::test]
async fn test_group_by() {
    let service = create_test_service().await;
    
    // 创建测试表
    let columns = vec![
        (0u32, "id", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, None),
        (1u32, "name", laoflchdb_engines::ColumnType::COLUMN_TYPE_STRING, None),
        (2u32, "department", laoflchdb_engines::ColumnType::COLUMN_TYPE_STRING, None),
        (3u32, "salary", laoflchdb_engines::ColumnType::COLUMN_TYPE_FLOAT, None),
    ];
    
    service.create_table("sys", "dept_test", None, &columns).await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
    
    // 插入测试数据
    let test_data = vec![
        (1, "Alice".to_string(), "IT".to_string(), 9500.0),
        (2, "Bob".to_string(), "HR".to_string(), 7500.0),
        (3, "Charlie".to_string(), "IT".to_string(), 8500.0),
        (4, "David".to_string(), "HR".to_string(), 7200.0),
        (5, "Eve".to_string(), "IT".to_string(), 9200.0),
    ];
    
    for (id, name, department, salary) in test_data {
        let mut row = Row::new();
        row.row_type = RowType::ROW_TYPE_NORMAL.into();
        row.version = 1;
        
        // id
        let mut f = laoflchdb_engines::Field::new();
        f.value = Some(Value::IntegerValue(Integer { value: id, special_fields: SpecialFields::default() }));
        let mut buf = Vec::new();
        { let mut os = CodedOutputStream::vec(&mut buf); f.write_to(&mut os).unwrap(); os.flush().unwrap(); }
        row.data.push(buf);
        
        // name
        let mut f = laoflchdb_engines::Field::new();
        f.value = Some(Value::StringValue(String { value: name, special_fields: SpecialFields::default() }));
        let mut buf = Vec::new();
        { let mut os = CodedOutputStream::vec(&mut buf); f.write_to(&mut os).unwrap(); os.flush().unwrap(); }
        row.data.push(buf);
        
        // department
        let mut f = laoflchdb_engines::Field::new();
        f.value = Some(Value::StringValue(String { value: department, special_fields: SpecialFields::default() }));
        let mut buf = Vec::new();
        { let mut os = CodedOutputStream::vec(&mut buf); f.write_to(&mut os).unwrap(); os.flush().unwrap(); }
        row.data.push(buf);
        
        // salary
        let mut f = laoflchdb_engines::Field::new();
        f.value = Some(Value::FloatValue(Float { value: salary, special_fields: SpecialFields::default() }));
        let mut buf = Vec::new();
        { let mut os = CodedOutputStream::vec(&mut buf); f.write_to(&mut os).unwrap(); os.flush().unwrap(); }
        row.data.push(buf);
        
        service.add_row("sys", "dept_test", &row).await.unwrap();
    }
    
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    // 测试 GROUP BY
    let result = service.sql_query("sys", "SELECT department, COUNT(*) FROM dept_test GROUP BY department").await;
    assert!(result.is_ok(), "GROUP BY 查询失败");
    let query_result = result.unwrap();
    assert!(query_result.rows.len() >= 1, "GROUP BY 应返回至少 1 行");
    
    // 测试带聚合的 GROUP BY
    let result = service.sql_query("sys", "SELECT department, SUM(salary) FROM dept_test GROUP BY department").await;
    assert!(result.is_ok(), "GROUP BY with SUM 查询失败");
    
    // 测试带 WHERE 的 GROUP BY
    let result = service.sql_query("sys", "SELECT department, AVG(salary) FROM dept_test WHERE salary > 7000 GROUP BY department").await;
    assert!(result.is_ok(), "GROUP BY with WHERE 查询失败");
    
    println!("GROUP BY 测试通过!");
}

/// 测试 ORDER BY
#[tokio::test]
async fn test_order_by() {
    let service = create_test_service().await;
    
    // 创建测试表
    let columns = vec![
        (0u32, "id", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, None),
        (1u32, "name", laoflchdb_engines::ColumnType::COLUMN_TYPE_STRING, None),
        (2u32, "age", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, None),
    ];
    
    service.create_table("sys", "order_test", None, &columns).await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
    
    // 插入测试数据
    let test_data = vec![
        (3, "Charlie".to_string(), 35),
        (1, "Alice".to_string(), 30),
        (5, "Eve".to_string(), 40),
        (2, "Bob".to_string(), 25),
        (4, "David".to_string(), 28),
    ];
    
    for (id, name, age) in test_data {
        let mut row = Row::new();
        row.row_type = RowType::ROW_TYPE_NORMAL.into();
        row.version = 1;
        
        let mut f = laoflchdb_engines::Field::new();
        f.value = Some(Value::IntegerValue(Integer { value: id, special_fields: SpecialFields::default() }));
        let mut buf = Vec::new();
        { let mut os = CodedOutputStream::vec(&mut buf); f.write_to(&mut os).unwrap(); os.flush().unwrap(); }
        row.data.push(buf);
        
        let mut f = laoflchdb_engines::Field::new();
        f.value = Some(Value::StringValue(String { value: name, special_fields: SpecialFields::default() }));
        let mut buf = Vec::new();
        { let mut os = CodedOutputStream::vec(&mut buf); f.write_to(&mut os).unwrap(); os.flush().unwrap(); }
        row.data.push(buf);
        
        let mut f = laoflchdb_engines::Field::new();
        f.value = Some(Value::IntegerValue(Integer { value: age, special_fields: SpecialFields::default() }));
        let mut buf = Vec::new();
        { let mut os = CodedOutputStream::vec(&mut buf); f.write_to(&mut os).unwrap(); os.flush().unwrap(); }
        row.data.push(buf);
        
        service.add_row("sys", "order_test", &row).await.unwrap();
    }
    
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    // 测试 ORDER BY id ASC
    let result = service.sql_query("sys", "SELECT * FROM order_test ORDER BY id ASC").await;
    assert!(result.is_ok(), "ORDER BY ASC 查询失败");
    
    // 测试 ORDER BY id DESC
    let result = service.sql_query("sys", "SELECT * FROM order_test ORDER BY id DESC").await;
    assert!(result.is_ok(), "ORDER BY DESC 查询失败");
    
    // 测试 ORDER BY 多个列
    let result = service.sql_query("sys", "SELECT * FROM order_test ORDER BY age DESC, id ASC").await;
    assert!(result.is_ok(), "ORDER BY 多列查询失败");
    
    println!("ORDER BY 测试通过!");
}

/// 测试 LIMIT 和 OFFSET
#[tokio::test]
async fn test_limit_offset() {
    let service = create_test_service().await;
    
    // 创建测试表
    let columns = vec![
        (0u32, "id", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, None),
        (1u32, "name", laoflchdb_engines::ColumnType::COLUMN_TYPE_STRING, None),
    ];
    
    service.create_table("sys", "limit_test", None, &columns).await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
    
    // 插入 10 条测试数据
    for i in 1..=10 {
        let mut row = Row::new();
        row.row_type = RowType::ROW_TYPE_NORMAL.into();
        row.version = 1;
        
        let mut f = laoflchdb_engines::Field::new();
        f.value = Some(Value::IntegerValue(Integer { value: i as i64, special_fields: SpecialFields::default() }));
        let mut buf = Vec::new();
        { let mut os = CodedOutputStream::vec(&mut buf); f.write_to(&mut os).unwrap(); os.flush().unwrap(); }
        row.data.push(buf);
        
        let mut f = laoflchdb_engines::Field::new();
        f.value = Some(Value::StringValue(String { value: format!("User{}", i), special_fields: SpecialFields::default() }));
        let mut buf = Vec::new();
        { let mut os = CodedOutputStream::vec(&mut buf); f.write_to(&mut os).unwrap(); os.flush().unwrap(); }
        row.data.push(buf);
        
        service.add_row("sys", "limit_test", &row).await.unwrap();
    }
    
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    // 测试 LIMIT 5
    let result = service.sql_query("sys", "SELECT * FROM limit_test LIMIT 5").await;
    assert!(result.is_ok(), "LIMIT 5 查询失败");
    let query_result = result.unwrap();
    assert_eq!(query_result.rows.len(), 5, "LIMIT 5 应返回 5 行");
    
    // 测试 LIMIT 3 OFFSET 2
    let result = service.sql_query("sys", "SELECT * FROM limit_test LIMIT 3 OFFSET 2").await;
    assert!(result.is_ok(), "LIMIT with OFFSET 查询失败");
    
    // 测试 ORDER BY + LIMIT
    let result = service.sql_query("sys", "SELECT * FROM limit_test ORDER BY id DESC LIMIT 3").await;
    assert!(result.is_ok(), "ORDER BY + LIMIT 查询失败");
    
    println!("LIMIT 和 OFFSET 测试通过!");
}

/// 测试复杂的嵌套查询
#[tokio::test]
async fn test_complex_queries() {
    let service = create_test_service().await;
    
    // 创建测试表
    let columns = vec![
        (0u32, "id", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, None),
        (1u32, "name", laoflchdb_engines::ColumnType::COLUMN_TYPE_STRING, None),
        (2u32, "age", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, None),
        (3u32, "score", laoflchdb_engines::ColumnType::COLUMN_TYPE_FLOAT, None),
    ];
    
    service.create_table("sys", "complex_test", None, &columns).await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
    
    // 插入测试数据
    let test_data = vec![
        (1, std::string::String::from("Alice"), 30, 95.5),
        (2, std::string::String::from("Bob"), 25, 88.0),
        (3, std::string::String::from("Charlie"), 35, 92.5),
        (4, std::string::String::from("David"), 28, 90.0),
        (5, std::string::String::from("Eve"), 40, 85.0),
    ];
    insert_test_data(&service, "complex_test", test_data).await.unwrap();
    
    // 测试: WHERE + ORDER BY + LIMIT
    let result = service.sql_query(
        "sys", 
        "SELECT * FROM complex_test WHERE age > 25 ORDER BY score DESC LIMIT 3"
    ).await;
    assert!(result.is_ok(), "WHERE + ORDER BY + LIMIT 查询失败");
    
    // 测试: 聚合 + GROUP BY + HAVING (如果有)
    let result = service.sql_query(
        "sys", 
        "SELECT age, COUNT(*) FROM complex_test GROUP BY age ORDER BY age"
    ).await;
    assert!(result.is_ok(), "聚合 + GROUP BY + ORDER BY 查询失败");
    
    // 测试: 多个条件的 AND/OR 组合
    let result = service.sql_query(
        "sys", 
        "SELECT * FROM complex_test WHERE (age > 25 AND age < 40) OR (score > 90 AND score < 95)"
    ).await;
    assert!(result.is_ok(), "复杂 AND/OR 组合查询失败");
    
    println!("复杂查询测试通过!");
}
