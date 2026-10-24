//! 图服务 REST 接口：`POST /graphql`
//!
//! 请求体：
//! ```json
//! { "query": "{ stats { nodeCount edgeCount } }", "variables": {}, "operationName": null }
//! ```
//! 响应体为 GraphQL over HTTP 标准格式：`{ "data": ..., "errors": [...] }`

use std::sync::Arc;

use axum::{extract::State, routing::post, Json, Router};
use serde::Deserialize;

use crate::GraphServiceImpl;

#[derive(Debug, Deserialize)]
pub struct GraphQLHttpRequest {
    pub query: String,
    #[serde(default)]
    pub variables: Option<serde_json::Value>,
    #[serde(default, alias = "operationName")]
    pub operation_name: Option<String>,
}

/// 创建 REST Router：`POST /graphql`。
/// 返回的 Router 直接合并到主 Router（[`axum::Router::merge`]），不额外嵌套前缀。
pub fn create_rest_router(service: Arc<GraphServiceImpl>) -> Router {
    Router::new()
        .route("/graphql", post(graphql_handler))
        .with_state(service)
}

async fn graphql_handler(
    State(service): State<Arc<GraphServiceImpl>>,
    Json(body): Json<GraphQLHttpRequest>,
) -> Json<serde_json::Value> {
    let outcome = service
        .execute_graphql(&body.query, body.variables, body.operation_name)
        .await;

    let mut response = serde_json::Map::new();
    response.insert("data".to_string(), outcome.data);
    if !outcome.errors.is_empty() {
        response.insert(
            "errors".to_string(),
            serde_json::Value::Array(outcome.errors),
        );
    }
    Json(serde_json::Value::Object(response))
}
