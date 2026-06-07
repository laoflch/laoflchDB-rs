//! lsql 客户端集成测试

use laoflchDB_rust::access::GrpcService;
use laoflchDB_rust::{DatabaseService, DatabaseServiceImpl};
use laoflchDB_rust::pb::rpc::laoflch_db_client::LaoflchDbClient;
use laoflchDB_rust::pb::rpc::*;
use std::sync::Arc;
use tokio::net::TcpListener;
use tonic::transport::Server;
use laoflchDB_rust::pb::rpc::laoflch_db_server::LaoflchDbServer;
use laoflchdb_engines;
use laoflchdb_engines::Field;
use protobuf::CodedOutputStream;
use protobuf::Message;

async fn setup_test_server() -> (String, tokio::task::JoinHandle<()>) {
    let temp_dir = std::env::temp_dir();
    let db_path = temp_dir.join(format!("test_lsql_{}", uuid::Uuid::new_v4()));
    let db_path_str = db_path.to_str().unwrap();
    
    let service = DatabaseServiceImpl::new(db_path_str).await;
    service.init_database().await.unwrap();
    
    let grpc_service = GrpcService::new(Arc::new(service));
    
    // 找一个可用的端口
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let addr_str = addr.to_string();
    
    // 启动服务器
    let handle = tokio::spawn(async move {
        Server::builder()
            .add_service(LaoflchDbServer::new(grpc_service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    
    // 等待一小会让服务器启动
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    (addr_str, handle)
}

async fn create_test_table(client: &mut LaoflchDbClient<tonic::transport::Channel>) {
    let create_req = CreateTableRequest {
        schema: "sys".to_string(),
        table_name: "test_users".to_string(),
        columns: vec![
            ColumnDef {
                name: "id".to_string(),
                column_type: 1, // INT64
            },
            ColumnDef {
                name: "name".to_string(),
                column_type: 0, // STRING
            },
            ColumnDef {
                name: "age".to_string(),
                column_type: 1, // INT64
            },
            ColumnDef {
                name: "score".to_string(),
                column_type: 3, // FLOAT
            },
        ],
    };
    
    client.create_table(create_req).await.unwrap();
}

fn build_row(id: i64, name: &str, age: i64, score: f64) -> Row {
    let mut row = Row::default();
    row.row_type = 0;
    row.version = 1;
    
    // 构建 id 字段
    let mut field1 = Field::new();
    field1.value = Some(laoflchdb_engines::field::Value::IntegerValue(
        laoflchdb_engines::field::Integer {
            value: id,
            special_fields: protobuf::SpecialFields::default(),
        }
    ));
    let mut buf1 = Vec::new();
    {
        let mut os = CodedOutputStream::vec(&mut buf1);
        field1.write_to(&mut os).unwrap();
        os.flush().unwrap();
    }
    row.data.push(buf1);
    
    // 构建 name 字段
    let mut field2 = Field::new();
    field2.value = Some(laoflchdb_engines::field::Value::StringValue(
        laoflchdb_engines::field::String {
            value: name.to_string(),
            special_fields: protobuf::SpecialFields::default(),
        }
    ));
    let mut buf2 = Vec::new();
    {
        let mut os = CodedOutputStream::vec(&mut buf2);
        field2.write_to(&mut os).unwrap();
        os.flush().unwrap();
    }
    row.data.push(buf2);
    
    // 构建 age 字段
    let mut field3 = Field::new();
    field3.value = Some(laoflchdb_engines::field::Value::IntegerValue(
        laoflchdb_engines::field::Integer {
            value: age,
            special_fields: protobuf::SpecialFields::default(),
        }
    ));
    let mut buf3 = Vec::new();
    {
        let mut os = CodedOutputStream::vec(&mut buf3);
        field3.write_to(&mut os).unwrap();
        os.flush().unwrap();
    }
    row.data.push(buf3);
    
    // 构建 score 字段
    let mut field4 = Field::new();
    field4.value = Some(laoflchdb_engines::field::Value::FloatValue(
        laoflchdb_engines::field::Float {
            value: score,
            special_fields: protobuf::SpecialFields::default(),
        }
    ));
    let mut buf4 = Vec::new();
    {
        let mut os = CodedOutputStream::vec(&mut buf4);
        field4.write_to(&mut os).unwrap();
        os.flush().unwrap();
    }
    row.data.push(buf4);
    
    row
}

async fn insert_test_data(client: &mut LaoflchDbClient<tonic::transport::Channel>) {
    let test_data = vec![
        (1, "Alice", 30, 95.5),
        (2, "Bob", 25, 88.0),
        (3, "Charlie", 35, 92.5),
    ];
    
    for (id, name, age, score) in test_data {
        let add_req = AddRowRequest {
            schema: "sys".to_string(),
            table_name: "test_users".to_string(),
            row: Some(build_row(id, name, age, score)),
        };
        
        client.add_row(add_req).await.unwrap();
    }
    
    // 等待数据写入
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
}

#[tokio::test]
async fn test_grpc_connection() {
    let (addr, _handle) = setup_test_server().await;
    
    // 测试连接
    let client_result = LaoflchDbClient::connect(format!("http://{}", addr)).await;
    assert!(client_result.is_ok(), "连接服务器失败");
}

#[tokio::test]
async fn test_list_tables() {
    let (addr, _handle) = setup_test_server().await;
    
    let mut client = LaoflchDbClient::connect(format!("http://{}", addr)).await.unwrap();
    
    // 创建测试表
    create_test_table(&mut client).await;
    
    // 列出表
    let list_req = ListTablesRequest {
        schema: "sys".to_string(),
    };
    
    let list_resp = client.list_tables(list_req).await.unwrap().into_inner();
    assert!(list_resp.success);
    assert!(list_resp.tables.contains(&"test_users".to_string()));
}

#[tokio::test]
async fn test_sql_query() {
    let (addr, _handle) = setup_test_server().await;
    
    let mut client = LaoflchDbClient::connect(format!("http://{}", addr)).await.unwrap();
    
    // 创建表和插入数据
    create_test_table(&mut client).await;
    insert_test_data(&mut client).await;
    
    // 测试 SELECT *
    let sql_req = SqlQueryRequest {
        schema: "sys".to_string(),
        sql: "SELECT * FROM test_users".to_string(),
    };
    
    let sql_resp = client.sql_query(sql_req).await.unwrap().into_inner();
    assert!(sql_resp.success);
    assert_eq!(sql_resp.rows.len(), 3);
    
    // 测试带 WHERE 条件的查询
    let sql_req = SqlQueryRequest {
        schema: "sys".to_string(),
        sql: "SELECT name FROM test_users WHERE age > 30".to_string(),
    };
    
    let sql_resp = client.sql_query(sql_req).await.unwrap().into_inner();
    assert!(sql_resp.success);
    assert_eq!(sql_resp.rows.len(), 1);
}

#[tokio::test]
async fn test_sql_aggregate() {
    let (addr, _handle) = setup_test_server().await;
    
    let mut client = LaoflchDbClient::connect(format!("http://{}", addr)).await.unwrap();
    
    // 创建表和插入数据
    create_test_table(&mut client).await;
    insert_test_data(&mut client).await;
    
    // 测试 COUNT
    let sql_req = SqlQueryRequest {
        schema: "sys".to_string(),
        sql: "SELECT COUNT(id) FROM test_users".to_string(),
    };
    
    let sql_resp = client.sql_query(sql_req).await.unwrap().into_inner();
    assert!(sql_resp.success);
    assert_eq!(sql_resp.rows.len(), 1);
    
    // 测试 AVG
    let sql_req = SqlQueryRequest {
        schema: "sys".to_string(),
        sql: "SELECT AVG(age) FROM test_users".to_string(),
    };
    
    let sql_resp = client.sql_query(sql_req).await.unwrap().into_inner();
    assert!(sql_resp.success);
}
