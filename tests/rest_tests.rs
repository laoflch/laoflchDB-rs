use laoflchDB_rust::access::RestService;
use laoflchDB_rust::{DatabaseService, DatabaseServiceImpl};
use laoflchDB_rust::db_engine::pb::ColumnType;
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
        axum::serve(listener, app).await.unwrap();
    });
    
    let client = reqwest::Client::new();
    let res = client.get(format!("http://{}/health", addr))
        .send()
        .await
        .unwrap();
    
    assert_eq!(res.status(), 200);
    let body = res.text().await.unwrap();
    assert!(body.contains("\"success\":true"));
}

#[tokio::test]
async fn test_rest_list_tables() {
    let (rest_service, _service) = setup_rest_service().await;
    let app = rest_service.router();
    
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    
    let client = reqwest::Client::new();
    let res = client.get(format!("http://{}/api/v1/schemas/sys/tables", addr))
        .send()
        .await
        .unwrap();
    
    assert_eq!(res.status(), 200);
    let body = res.text().await.unwrap();
    assert!(body.contains("\"success\":true"));
}

#[tokio::test]
async fn test_rest_get_table_meta() {
    let (rest_service, _service) = setup_rest_service().await;
    let app = rest_service.router();
    
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    
    let client = reqwest::Client::new();
    let res = client.get(format!("http://{}/api/v1/schemas/sys/tables/user", addr))
        .send()
        .await
        .unwrap();
    
    assert_eq!(res.status(), 200);
    let body = res.text().await.unwrap();
    assert!(body.contains("\"success\":true"));
    assert!(body.contains("\"table_name\":\"user\""));
}

#[tokio::test]
async fn test_rest_put_and_get() {
    let (rest_service, service) = setup_rest_service().await;
    let app = rest_service.router();
    
    service.create_table("sys", "test_data", &[
        (0, "id", ColumnType::Int64),
        (1, "name", ColumnType::String),
    ]).await.unwrap();
    
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    
    let client = reqwest::Client::new();
    
    let put_body = serde_json::json!({
        "schema": "sys",
        "table": "test_data",
        "key": "74657374",
        "value": "68656c6c6f"
    });
    
    let res = client.post(format!("http://{}/api/v1/put", addr))
        .json(&put_body)
        .send()
        .await
        .unwrap();
    
    assert_eq!(res.status(), 200);
    let res_body = res.text().await.unwrap();
    assert!(res_body.contains("\"success\":true"), "Response: {}", res_body);
}

#[tokio::test]
async fn test_rest_create_table() {
    let (rest_service, _service) = setup_rest_service().await;
    let app = rest_service.router();
    
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    
    let client = reqwest::Client::new();
    
    let body = serde_json::json!({
        "schema": "sys",
        "table_name": "test_table",
        "columns": [
            {"name": "id", "column_type": "INT64"},
            {"name": "name", "column_type": "STRING"}
        ]
    });
    
    let res = client.post(format!("http://{}/api/v1/tables", addr))
        .json(&body)
        .send()
        .await
        .unwrap();
    
    assert_eq!(res.status(), 200);
    let res_body = res.text().await.unwrap();
    assert!(res_body.contains("\"success\":true"));
    assert!(res_body.contains("\"table_id\":"));
}
