use crate::service::DatabaseService;
use crate::db_engine::pb::ColumnType;
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use axum::{
    Router,
    routing::{get, post},
    extract::{Path, Query, State},
    response::Json,
};

#[derive(Clone)]
pub struct RestService {
    service: Arc<dyn DatabaseService>,
}

impl RestService {
    pub fn new(service: Arc<dyn DatabaseService>) -> Self {
        Self { service }
    }

    pub fn router(&self) -> Router {
        Router::new()
            .route("/health", get(health_handler))
            .route("/api/v1/get", get(get_handler))
            .route("/api/v1/put", post(put_handler))
            .route("/api/v1/delete", post(delete_handler))
            .route("/api/v1/tables", post(create_table_handler))
            .route("/api/v1/schemas/:schema/tables", get(list_tables_handler))
            .route("/api/v1/schemas/:schema/tables/:table", get(get_table_meta_handler))
            .with_state(self.service.clone())
    }

    pub async fn start(&self, addr: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        log::info!("REST server listening on {}", addr);
        axum::serve(listener, self.router()).await?;
        Ok(())
    }
}

#[derive(Serialize)]
pub struct ApiResponse<T> {
    success: bool,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<T>,
}

impl<T> ApiResponse<T> {
    pub fn success(data: T) -> Self {
        Self {
            success: true,
            message: String::new(),
            data: Some(data),
        }
    }

    pub fn error(message: String) -> Self {
        Self {
            success: false,
            message,
            data: None,
        }
    }
}

#[derive(Deserialize, Debug)]
pub struct GetQuery {
    pub schema: Option<String>,
    pub table: String,
    pub key: String,
}

#[derive(Deserialize, Debug)]
pub struct PutBody {
    pub schema: Option<String>,
    pub table: String,
    pub key: String,
    pub value: String,
}

#[derive(Deserialize, Debug)]
pub struct DeleteBody {
    pub schema: Option<String>,
    pub table: String,
    pub key: String,
}

#[derive(Deserialize, Debug)]
pub struct CreateTableBody {
    pub schema: Option<String>,
    pub table_name: String,
    pub columns: Vec<ColumnDefinition>,
}

#[derive(Deserialize, Debug)]
pub struct ColumnDefinition {
    pub name: String,
    pub column_type: String,
}

#[derive(Serialize)]
pub struct CreateTableResponse {
    pub table_id: u64,
}

#[derive(Serialize)]
pub struct TableMetaResponse {
    pub table_id: u64,
    pub table_name: String,
    pub column_count: u32,
}

#[derive(Serialize)]
pub struct GetResponse {
    pub value: Option<Vec<u8>>,
}

fn decode_hex(s: &str) -> Result<Vec<u8>, String> {
    if s.len() % 2 == 0 && s.chars().all(|c| c.is_ascii_hexdigit()) {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i+2], 16).map_err(|e| e.to_string()))
            .collect()
    } else {
        Ok(s.as_bytes().to_vec())
    }
}

async fn health_handler() -> Json<ApiResponse<&'static str>> {
    Json(ApiResponse::success("OK"))
}

async fn get_handler(
    State(service): State<Arc<dyn DatabaseService>>,
    Query(query): Query<GetQuery>,
) -> Json<ApiResponse<GetResponse>> {
    let schema = query.schema.unwrap_or_else(|| "sys".to_string());
    let key = match decode_hex(&query.key) {
        Ok(k) => k,
        Err(e) => return Json(ApiResponse::error(e)),
    };
    
    match service.get(&schema, &query.table, &key) {
        Ok(value) => Json(ApiResponse::success(GetResponse { value })),
        Err(e) => Json(ApiResponse::error(e.to_string())),
    }
}

async fn put_handler(
    State(service): State<Arc<dyn DatabaseService>>,
    Json(body): Json<PutBody>,
) -> Json<ApiResponse<&'static str>> {
    let schema = body.schema.unwrap_or_else(|| "sys".to_string());
    let key = match decode_hex(&body.key) {
        Ok(k) => k,
        Err(e) => return Json(ApiResponse::error(e)),
    };
    let value = match decode_hex(&body.value) {
        Ok(v) => v,
        Err(e) => return Json(ApiResponse::error(e)),
    };
    
    match service.put(&schema, &body.table, &key, &value) {
        Ok(()) => Json(ApiResponse::success("OK")),
        Err(e) => Json(ApiResponse::error(e.to_string())),
    }
}

async fn delete_handler(
    State(service): State<Arc<dyn DatabaseService>>,
    Json(body): Json<DeleteBody>,
) -> Json<ApiResponse<&'static str>> {
    let schema = body.schema.unwrap_or_else(|| "sys".to_string());
    let key = match decode_hex(&body.key) {
        Ok(k) => k,
        Err(e) => return Json(ApiResponse::error(e)),
    };
    
    match service.delete(&schema, &body.table, &key) {
        Ok(()) => Json(ApiResponse::success("OK")),
        Err(e) => Json(ApiResponse::error(e.to_string())),
    }
}

async fn create_table_handler(
    State(service): State<Arc<dyn DatabaseService>>,
    Json(body): Json<CreateTableBody>,
) -> Json<ApiResponse<CreateTableResponse>> {
    let schema = body.schema.unwrap_or_else(|| "sys".to_string());
    
    let columns: Vec<(u32, &str, ColumnType)> = body.columns
        .iter()
        .enumerate()
        .map(|(idx, col)| {
            let ct = match col.column_type.to_uppercase().as_str() {
                "STRING" => ColumnType::String,
                "INT64" | "INT" => ColumnType::Int64,
                "BYTES" | "BINARY" => ColumnType::Bytes,
                "FLOAT" | "DOUBLE" => ColumnType::Float,
                "LIST" => ColumnType::List,
                "IMAGE" => ColumnType::Image,
                _ => ColumnType::String,
            };
            (idx as u32, col.name.as_str(), ct)
        })
        .collect();
    
    match service.create_table(&schema, &body.table_name, &columns) {
        Ok(table_id) => Json(ApiResponse::success(CreateTableResponse { table_id })),
        Err(e) => Json(ApiResponse::error(e.to_string())),
    }
}

async fn list_tables_handler(
    State(service): State<Arc<dyn DatabaseService>>,
    Path(schema): Path<String>,
) -> Json<ApiResponse<Vec<String>>> {
    match service.list_tables(&schema) {
        Ok(tables) => Json(ApiResponse::success(tables)),
        Err(e) => Json(ApiResponse::error(e.to_string())),
    }
}

async fn get_table_meta_handler(
    State(service): State<Arc<dyn DatabaseService>>,
    Path((schema, table)): Path<(String, String)>,
) -> Json<ApiResponse<TableMetaResponse>> {
    match service.get_table_meta(&schema, &table) {
        Ok(Some(meta)) => {
            let response = TableMetaResponse {
                table_id: meta.table_id,
                table_name: meta.table_name,
                column_count: meta.column_count,
            };
            Json(ApiResponse::success(response))
        },
        Ok(None) => Json(ApiResponse::error("Table not found".to_string())),
        Err(e) => Json(ApiResponse::error(e.to_string())),
    }
}
