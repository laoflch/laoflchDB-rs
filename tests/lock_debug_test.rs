//! 锁竞争调试测试

use laoflchDB_rust::{DatabaseService, DatabaseServiceImpl};
use laoflchdb_engines::{Row, RowType, field::field::Value, field::{String, Integer}, Message};
use protobuf::CodedOutputStream;
use std::sync::Arc;
use std::time::{Instant, Duration};

async fn create_test_service() -> Arc<dyn DatabaseService> {
    let temp_dir = std::env::temp_dir();
    let db_path = temp_dir.join(format!("test_lock_debug_{}", uuid::Uuid::new_v4()));
    let db_path_str = db_path.to_str().unwrap();
    
    let service = DatabaseServiceImpl::new(db_path_str).await;
    service.init_database().await.unwrap();
    Arc::new(service)
}

#[tokio::test]
async fn test_sql_engine_lock_contention() {
    let service = create_test_service().await;
    
    // 创建测试表
    let columns = vec![
        (0u32, "id", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, None),
        (1u32, "dept", laoflchdb_engines::ColumnType::COLUMN_TYPE_STRING, None),
    ];
    
    service.create_table("sys", "lock_test", None, &columns).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // 插入测试数据
    let depts = ["IT", "HR", "IT", "HR", "IT"];
    for i in 0..5 {
        let mut row = Row::new();
        row.row_type = RowType::ROW_TYPE_NORMAL.into();
        row.version = 1;
        
        let mut f = laoflchdb_engines::Field::new();
        f.value = Some(Value::IntegerValue(Integer { value: (i+1) as i64, special_fields: Default::default() }));
        let mut buf = Vec::new();
        { let mut os = CodedOutputStream::vec(&mut buf); f.write_to(&mut os).unwrap(); os.flush().unwrap(); }
        row.data.push(buf);
        
        let mut f = laoflchdb_engines::Field::new();
        f.value = Some(Value::StringValue(String { value: depts[i].to_string(), special_fields: Default::default() }));
        let mut buf = Vec::new();
        { let mut os = CodedOutputStream::vec(&mut buf); f.write_to(&mut os).unwrap(); os.flush().unwrap(); }
        row.data.push(buf);
        
        service.add_row("sys", "lock_test", &row).await.unwrap();
    }
    
    tokio::time::sleep(Duration::from_millis(200)).await;
    
    // 测试1: 先执行一个可能卡住的查询，然后尝试创建表
    println!("测试1: 执行 GROUP BY 查询...");
    let service_clone = service.clone();
    
    let handle = tokio::spawn(async move {
        let start = Instant::now();
        let result = tokio::time::timeout(Duration::from_secs(5), service_clone.sql_query("sys", "SELECT dept, COUNT(id) FROM lock_test GROUP BY dept")).await;
        println!("GROUP BY 查询耗时: {:?}", start.elapsed());
        result
    });
    
    // 等待查询开始执行
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // 尝试创建新表（需要写锁）
    println!("测试2: 在查询执行期间尝试创建表...");
    let start = Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(3), service.create_table("sys", "new_table", None, &[(0u32, "id", laoflchdb_engines::ColumnType::COLUMN_TYPE_INT64, None)])).await;
    println!("创建表耗时: {:?}", start.elapsed());
    
    match result {
        Ok(Ok(_)) => println!("创建表成功"),
        Ok(Err(e)) => println!("创建表失败: {}", e),
        Err(_) => println!("创建表超时 - 可能存在锁竞争!"),
    }
    
    // 等待查询完成
    let _ = handle.await;
}
