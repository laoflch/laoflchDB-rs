use laoflchDB_rust::{
    service::index::{IndexService, IndexServiceImpl},
    AccessService,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tempfile::TempDir;

#[derive(Serialize)]
struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Deserialize)]
struct LoginResponse {
    success: bool,
    token: Option<String>,
}

#[derive(Serialize)]
struct CreateIndexRequest {
    index_name: String,
    fields: Vec<IndexFieldDefinition>,
}

#[derive(Serialize)]
struct IndexFieldDefinition {
    name: String,
    field_type: String,
    comment: Option<String>,
}

#[derive(Deserialize)]
struct ApiResponse<T> {
    success: bool,
    data: Option<T>,
    message: String,
}

#[derive(Deserialize)]
struct CreateIndexResponse {
    index_id: u64,
}

#[derive(Deserialize)]
struct IndexListResponse {
    indices: Vec<String>,
}

#[derive(Deserialize)]
struct IndexStatsResponse {
    total_indices: usize,
    index_names: Vec<String>,
}

async fn setup_test_server() -> (String, String, tempfile::TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let base_path = temp_dir.path().to_str().unwrap().to_string();
    
    // 创建 IndexService
    let index_service = Arc::new(IndexServiceImpl::new(&base_path, "test_search").await.unwrap());
    
    // 创建 AccessService 并设置 IndexService
    // 注意：这里我们使用一个空的 DatabaseService 因为主要测试 Index 功能
    // 在实际测试中应该提供完整的 DatabaseService
    let access_service = AccessService::new(Arc::new(MockDatabaseService::new()))
        .with_index_service(index_service);
    
    let rest_service = access_service.get_rest_service(None);
    
    // 启动服务器
    let addr = "127.0.0.1:0".to_string();
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server_addr = format!("http://127.0.0.1:{}", port);
    
    tokio::spawn(async move {
        axum::serve(listener, rest_service.router()).await.unwrap();
    });
    
    // 等待服务器启动
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    // 登录获取 token
    let client = reqwest::Client::new();
    let login_resp: ApiResponse<LoginResponse> = client
        .post(format!("{}/api/v1/login", server_addr))
        .json(&LoginRequest {
            username: "admin".to_string(),
            password: "laoflchdb".to_string(),
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    
    let token = login_resp.data.unwrap().token.unwrap();
    
    (server_addr, token, temp_dir)
}

// 简单的 Mock DatabaseService 用于测试
use async_trait::async_trait;
use laoflchDB_rust::service::DatabaseService;
use laoflchdb_engines::{ColumnType, ColumnMeta, Query, QueryResult, Row, TableMeta};

struct MockDatabaseService;

impl MockDatabaseService {
    fn new() -> Self {
        Self
    }
}

#[async_trait]
impl DatabaseService for MockDatabaseService {
    async fn init_database(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    async fn create_schema(&self, _schema: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    async fn list_schemas(&self) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(vec!["sys".to_string()])
    }

    async fn drop_schema(&self, _schema: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    async fn create_table(&self, _schema: &str, _table: &str, _table_comment: Option<&str>, _columns: &[(u32, &str, ColumnType, Option<&str>)]) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        Ok(1)
    }

    async fn drop_table(&self, _schema: &str, _table: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    async fn list_tables(&self, _schema: &str) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(vec![])
    }

    async fn list_table_cols(&self, _schema: &str, _table: &str) -> Result<Vec<ColumnMeta>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(vec![])
    }

    async fn update_table_comment(&self, _schema: &str, _table: &str, _comment: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    async fn update_column_comment(&self, _schema: &str, _table: &str, _column_name: &str, _comment: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    async fn add_row(&self, _schema: &str, _table: &str, _row: &Row) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        Ok(1)
    }

    async fn get_row(&self, _schema: &str, _table: &str, _row_id: u64) -> Result<Option<Row>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(None)
    }

    async fn delete_row(&self, _schema: &str, _table: &str, _row_id: u64) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    async fn update_row(&self, _schema: &str, _table: &str, _row_id: u64, _row: &Row) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    async fn query(&self, _schema: &str, _query: &Query) -> Result<QueryResult, Box<dyn std::error::Error + Send + Sync>> {
        Ok(QueryResult { rows: vec![], columns: vec![] })
    }

    async fn get_all_meta(&self, _schema: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok("{}".to_string())
    }

    async fn get_schema_info(&self, _schema: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok("{}".to_string())
    }

    async fn get_table_meta(&self, _schema: &str, _table: &str) -> Result<Option<TableMeta>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(None)
    }

    async fn put(&self, _schema: &str, _table: &str, _key: &[u8], _value: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    async fn get(&self, _schema: &str, _table: &str, _key: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(None)
    }

    async fn shutdown(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    async fn delete(&self, _schema: &str, _table: &str, _key: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    async fn sql_query(&self, _schema: &str, _sql: &str) -> Result<QueryResult, Box<dyn std::error::Error + Send + Sync>> {
        Ok(QueryResult { rows: vec![], columns: vec![] })
    }

    async fn refresh_tables(&self, _schema: &str) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(vec![])
    }
}

#[tokio::test]
async fn test_index_create_and_list() {
    let (server_addr, token, _temp_dir) = setup_test_server().await;
    let client = reqwest::Client::new();
    
    // 创建索引
    let create_req = CreateIndexRequest {
        index_name: "test_articles".to_string(),
        fields: vec![
            IndexFieldDefinition {
                name: "title".to_string(),
                field_type: "STRING".to_string(),
                comment: Some("Article title".to_string()),
            },
            IndexFieldDefinition {
                name: "content".to_string(),
                field_type: "STRING".to_string(),
                comment: Some("Article content".to_string()),
            },
        ],
    };
    
    let create_resp: ApiResponse<CreateIndexResponse> = client
        .post(format!("{}/api/v1/index/indices", server_addr))
        .header("Authorization", format!("Bearer {}", token))
        .json(&create_req)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    
    assert!(create_resp.success);
    assert!(create_resp.data.unwrap().index_id > 0);
    
    // 列出索引
    let list_resp: ApiResponse<IndexListResponse> = client
        .get(format!("{}/api/v1/index/indices", server_addr))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    
    assert!(list_resp.success);
    let indices = list_resp.data.unwrap().indices;
    assert_eq!(indices.len(), 1);
    assert!(indices.contains(&"test_articles".to_string()));
}

#[tokio::test]
async fn test_index_stats() {
    let (server_addr, token, _temp_dir) = setup_test_server().await;
    let client = reqwest::Client::new();
    
    // 获取初始统计
    let stats_resp: ApiResponse<IndexStatsResponse> = client
        .get(format!("{}/api/v1/index/stats", server_addr))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    
    assert!(stats_resp.success);
    assert_eq!(stats_resp.data.unwrap().total_indices, 0);
    
    // 创建索引
    let create_req = CreateIndexRequest {
        index_name: "test_index".to_string(),
        fields: vec![
            IndexFieldDefinition {
                name: "field1".to_string(),
                field_type: "STRING".to_string(),
                comment: None,
            },
        ],
    };
    
    let _: ApiResponse<CreateIndexResponse> = client
        .post(format!("{}/api/v1/index/indices", server_addr))
        .header("Authorization", format!("Bearer {}", token))
        .json(&create_req)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    
    // 获取更新后的统计
    let stats_resp: ApiResponse<IndexStatsResponse> = client
        .get(format!("{}/api/v1/index/stats", server_addr))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    
    assert!(stats_resp.success);
    let stats = stats_resp.data.unwrap();
    assert_eq!(stats.total_indices, 1);
    assert!(stats.index_names.contains(&"test_index".to_string()));
}

#[tokio::test]
async fn test_index_unauthorized_access() {
    let (server_addr, _token, _temp_dir) = setup_test_server().await;
    let client = reqwest::Client::new();
    
    // 尝试不带 token 访问
    let resp = client
        .get(format!("{}/api/v1/index/indices", server_addr))
        .send()
        .await
        .unwrap();
    
    assert_eq!(resp.status(), 403);
}

#[tokio::test]
async fn test_index_invalid_token() {
    let (server_addr, _token, _temp_dir) = setup_test_server().await;
    let client = reqwest::Client::new();
    
    // 尝试使用无效 token
    let resp = client
        .get(format!("{}/api/v1/index/indices", server_addr))
        .header("Authorization", "Bearer invalid_token")
        .send()
        .await
        .unwrap();
    
    assert_eq!(resp.status(), 403);
}
