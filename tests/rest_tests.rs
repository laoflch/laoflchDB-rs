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
    
    service.create_schema("test").await.unwrap();
    
    let resp = client.post(&format!("http://{}/schemas/test/tables", addr))
        .json(&serde_json::json!({
            "table_name": "users",
            "columns": [
                {"column_id": 1, "column_name": "id", "column_type": "INT64"},
                {"column_id": 2, "column_name": "name", "column_type": "STRING"}
            ]
        }))
        .send()
        .await
        .expect("request failed");
    
    assert_eq!(resp.status(), 200);
}
