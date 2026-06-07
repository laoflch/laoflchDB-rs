//! DataFusion 调试测试
//! 分析 GROUP BY 和 ORDER BY 卡住的原因

use laoflchDB_rust::{DatabaseService, DatabaseServiceImpl};
use laoflchdb_engines::{Row, RowType, field::field::Value, field::{String, Integer}, Message};
use protobuf::CodedOutputStream;
use std::sync::Arc;
use std::time::Instant;

async fn create_test_service() -> Arc<dyn DatabaseService> {
    let temp_dir = std::env::temp_dir();
    let db_path = temp_dir.join(format!("test_df_debug_{}", uuid::Uuid::new_v4()));
    let db_path_str = db_path.to_str().unwrap();
    
    let service = DatabaseServiceImpl::new(db_path_str).await;
    service.init_database().await.unwrap();
    Arc::new(service)
}

#[tokio::test]
async fn test_group_by_debug() {
    let service = create_test_service().await;
    
    // 创建测试表
    let columns = vec![
        (0u32, "id", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64),
        (1u32, "dept", laoflchdb_engines::ColumnType::COLUMN_TYPE_STRING),
        (2u32, "salary", laoflchdb_engines::ColumnType::COLUMN_TYPE_FLOAT),
    ];
    
    service.create_table("sys", "debug_test", &columns).await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    
    // 插入测试数据
    let depts = ["IT", "HR", "IT", "HR", "IT"];
    for i in 0..5 {
        let mut row = Row::new();
        row.row_type = RowType::ROW_TYPE_NORMAL.into();
        row.version = 1;
        
        // id
        let mut f = laoflchdb_engines::Field::new();
        f.value = Some(Value::IntegerValue(Integer { value: (i+1) as i64, special_fields: Default::default() }));
        let mut buf = Vec::new();
        { let mut os = CodedOutputStream::vec(&mut buf); f.write_to(&mut os).unwrap(); os.flush().unwrap(); }
        row.data.push(buf);
        
        // dept
        let mut f = laoflchdb_engines::Field::new();
        f.value = Some(Value::StringValue(String { value: depts[i].to_string(), special_fields: Default::default() }));
        let mut buf = Vec::new();
        { let mut os = CodedOutputStream::vec(&mut buf); f.write_to(&mut os).unwrap(); os.flush().unwrap(); }
        row.data.push(buf);
        
        // salary
        let mut f = laoflchdb_engines::Field::new();
        f.value = Some(Value::FloatValue(laoflchdb_engines::field::Float { value: 1000.0 * (i+1) as f64, special_fields: Default::default() }));
        let mut buf = Vec::new();
        { let mut os = CodedOutputStream::vec(&mut buf); f.write_to(&mut os).unwrap(); os.flush().unwrap(); }
        row.data.push(buf);
        
        service.add_row("sys", "debug_test", &row).await.unwrap();
    }
    
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    
    // 测试简单查询（应该正常）
    println!("测试简单查询...");
    let start = Instant::now();
    let result = service.sql_query("sys", "SELECT * FROM debug_test").await;
    println!("简单查询耗时: {:?}", start.elapsed());
    assert!(result.is_ok(), "简单查询失败");
    
    // 测试 COUNT(column)（应该正常）
    println!("\n测试 COUNT(id)...");
    let start = Instant::now();
    let result = service.sql_query("sys", "SELECT COUNT(id) FROM debug_test").await;
    println!("COUNT 查询耗时: {:?}", start.elapsed());
    assert!(result.is_ok(), "COUNT 查询失败");
    
    // 测试 GROUP BY（可能卡住）
    println!("\n测试 GROUP BY...");
    let start = Instant::now();
    let result = tokio::time::timeout(tokio::time::Duration::from_secs(10), service.sql_query("sys", "SELECT dept, COUNT(id) FROM debug_test GROUP BY dept")).await;
    println!("GROUP BY 查询耗时: {:?}", start.elapsed());
    
    match result {
        Ok(Ok(_)) => println!("GROUP BY 查询成功"),
        Ok(Err(e)) => println!("GROUP BY 查询失败: {}", e),
        Err(_) => println!("GROUP BY 查询超时！"),
    }
    
    // 测试 ORDER BY（可能卡住）
    println!("\n测试 ORDER BY...");
    let start = Instant::now();
    let result = tokio::time::timeout(tokio::time::Duration::from_secs(10), service.sql_query("sys", "SELECT * FROM debug_test ORDER BY id")).await;
    println!("ORDER BY 查询耗时: {:?}", start.elapsed());
    
    match result {
        Ok(Ok(_)) => println!("ORDER BY 查询成功"),
        Ok(Err(e)) => println!("ORDER BY 查询失败: {}", e),
        Err(_) => println!("ORDER BY 查询超时！"),
    }
}
