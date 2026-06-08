use laoflchDB_rust::access::RestService;
use laoflchDB_rust::{DatabaseService, DatabaseServiceImpl};
use laoflchdb_engines::ColumnType;
use std::sync::Arc;
use tokio::net::TcpListener;

async fn create_test_service() -> Arc<dyn DatabaseService> {
    let temp_dir = std::env::temp_dir();
    let db_path = temp_dir.join(format!("test_rest_{}", uuid::Uuid::new_v4()));
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
async fn test_rest_health() {
    let (rest_service, _service) = setup_rest_service().await;
    let app = rest_service.router();
    
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    
    tokio::spawn(async move {
        axum::serve(listener, app).await;
    });
    
    let client = reqwest::Client::new();
    let resp = client.get(&format!("http://{}/health", addr))
        .send()
        .await
        .expect("request failed");
    
    assert_eq!(resp.status(), 200);
}

async fn login(client: &reqwest::Client, addr: std::net::SocketAddr) -> String {
    let login_body = serde_json::json!({
        "username": "admin",
        "password": "laoflchdb"
    });
    
    let res = client.post(format!("http://{}/api/v1/login", addr))
        .json(&login_body)
        .send()
        .await
        .unwrap();
    
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    body["data"]["token"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn test_rest_create_table() {
    let (rest_service, service) = setup_rest_service().await;
    let app = rest_service.router();
    
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    
    tokio::spawn(async move {
        axum::serve(listener, app).await;
    });
    
    let client = reqwest::Client::new();
    
    // 登录获取 token
    let token = login(&client, addr).await;
    let auth_header = format!("Bearer {}", token);
    
    service.create_schema("test").await.unwrap();
    
    let resp = client.post(&format!("http://{}/api/v1/tables", addr))
        .header("Authorization", &auth_header)
        .json(&serde_json::json!({
            "schema": "test",
            "table_name": "users",
            "columns": [
                {"name": "id", "column_type": "Int64"},
                {"name": "name", "column_type": "String"}
            ]
        }))
        .send()
        .await
        .expect("request failed");
    
    assert_eq!(resp.status(), 200);
}
