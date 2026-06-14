use crate::service::DatabaseService;
use crate::service::index::IndexService;
use crate::access::{PermissionChecker, PermissionContext, TokenManager};
use crate::config::PermissionAction;
use protobuf::Enum;
use laoflchdb_engines::{ColumnType, ColumnMeta, Row, SpecialFields, EnumOrUnknown, RowType};
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use axum::{
    Router,
    routing::{get, post, delete, put},
    extract::{Path, Query, State},
    response::Json,
    http::{StatusCode, Request},
    response::IntoResponse,
    middleware::{self},
};


#[derive(Clone)]
pub struct RestService {
    service: Arc<dyn DatabaseService>,
    index_service: Option<Arc<dyn IndexService>>,
    permission_checker: Arc<PermissionChecker>,
    service_id: String,
    token_manager: Arc<TokenManager>,
}

impl RestService {
    pub fn new(service: Arc<dyn DatabaseService>) -> Self {
        Self {
            service,
            index_service: None,
            permission_checker: Arc::new(PermissionChecker::new(true)),
            service_id: "default".to_string(),
            token_manager: Arc::new(TokenManager::default()),
        }
    }

    pub fn with_permissions(service: Arc<dyn DatabaseService>, permission_checker: Arc<PermissionChecker>, service_id: String) -> Self {
        Self {
            service,
            index_service: None,
            permission_checker,
            service_id,
            token_manager: Arc::new(TokenManager::default()),
        }
    }

    pub fn with_token_manager(service: Arc<dyn DatabaseService>, permission_checker: Arc<PermissionChecker>, service_id: String, token_manager: Arc<TokenManager>) -> Self {
        Self {
            service,
            index_service: None,
            permission_checker,
            service_id,
            token_manager,
        }
    }

    /// 设置 IndexService
    pub fn with_index_service(mut self, index_service: Arc<dyn IndexService>) -> Self {
        self.index_service = Some(index_service);
        self
    }

    pub fn router(&self) -> Router {
        let state = (self.service.clone(), self.permission_checker.clone(), self.service_id.clone(), self.token_manager.clone());
        
        // 构建需要认证的基础路由
        let auth_router = Router::new()
            .route("/logout", post(logout_handler))
            // KV 操作
            .route("/get", get(get_handler))
            .route("/put", post(put_handler))
            .route("/delete", post(delete_kv_handler))
            // 表管理
            .route("/tables", post(create_table_handler))
            .route("/schemas/:schema/tables/:table", delete(drop_table_handler))
            .route("/schemas/:schema/tables", get(list_tables_handler))
            .route("/schemas/:schema/tables/:table/columns", get(list_table_cols_handler))
            .route("/schemas/:schema/tables/:table", get(get_table_meta_handler))
            // 行操作
            .route("/schemas/:schema/tables/:table/rows", post(add_row_handler))
            .route("/schemas/:schema/tables/:table/rows/:row_id", get(get_row_handler))
            .route("/schemas/:schema/tables/:table/rows/:row_id", delete(delete_row_handler))
            .route("/schemas/:schema/tables/:table/rows/:row_id", put(update_row_handler))
            // 元数据查询
            .route("/schemas/:schema/meta", get(get_all_meta_handler))
            .route("/schemas/:schema/info", get(get_schema_info_handler))
            // SQL 查询
            .route("/sql_query", post(sql_query_handler))
            .layer(axum::middleware::from_fn_with_state(state.clone(), auth_middleware));
        
        let mut main_router = Router::new()
            // 公开路由（不需要认证）
            .route("/health", get(health_handler))
            .route("/api/v1/login", post(login_handler))
            // 需要认证的基础路由
            .nest("/api/v1", auth_router)
            .with_state(state);
        
        // 如果设置了 IndexService，添加索引路由
        if let Some(ref index_svc) = self.index_service {
            let index_state = (Arc::clone(index_svc), self.permission_checker.clone(), self.service_id.clone(), self.token_manager.clone());
            let index_router = Router::new()
                .route("/indices", post(create_index_handler))
                .route("/indices", get(list_indices_handler))
                .route("/indices/:index_name", delete(drop_index_handler))
                .route("/indices/:index_name/fields", get(get_index_fields_handler))
                .route("/indices/:index_name/meta", get(get_index_meta_handler))
                .route("/indices/:index_name/docs", post(add_document_handler))
                .route("/indices/:index_name/docs/:doc_id", delete(delete_document_handler))
                .route("/indices/:index_name/search", get(search_handler))
                .route("/indices/:index_name/search/multi", post(search_multi_field_handler))
                .route("/stats", get(get_index_stats_handler))
                .layer(axum::middleware::from_fn_with_state(index_state.clone(), index_auth_middleware))
                .with_state(index_state);
            
            main_router = main_router.nest("/api/v1/index", index_router);
        }
        
        main_router
    }

    pub async fn start(&self, addr: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        log::info!("REST server listening on {}", addr);
        axum::serve(listener, self.router()).await?;
        Ok(())
    }
}

type SharedState = (Arc<dyn DatabaseService>, Arc<PermissionChecker>, String, Arc<TokenManager>);

async fn auth_middleware(
    State((_, _, _, token_manager)): State<SharedState>,
    req: Request<axum::body::Body>,
    next: middleware::Next,
) -> Result<axum::response::Response, ApiError> {
    let auth_header = req.headers().get(axum::http::header::AUTHORIZATION);
    
    if let Some(header) = auth_header {
        let header_str = header.to_str().map_err(|_| ApiError { message: "Invalid authorization header".to_string() })?;
        
        if header_str.starts_with("Bearer ") {
            let token = &header_str[7..];
            if token_manager.validate_token(token).await.is_some() {
                return Ok(next.run(req).await);
            }
        }
    }
    
    Err(ApiError { message: "Unauthorized: Invalid or missing token".to_string() })
}

pub struct ApiError {
    message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let body = ApiResponse::<()>::error(self.message);
        (StatusCode::FORBIDDEN, Json(body)).into_response()
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

use sha2::{Sha256, Digest};

fn hash_password(password: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}

async fn verify_user(service: &Arc<dyn DatabaseService>, username: &str, password: &str) -> Result<i64, String> {
    let result = service
        .sql_query("sys", &format!(
            "SELECT id, password_hash FROM user WHERE username = '{}'",
            username.replace("'", "''")
        ))
        .await
        .map_err(|e| format!("Database query failed: {}", e))?;

    if result.rows.is_empty() {
        return Err("Invalid username or password".to_string());
    }

    let qr = &result.rows[0];
    if !qr.row.is_some() {
        return Err("User data error".to_string());
    }

    let row = qr.row.get_or_default();
    if row.data.len() < 2 {
        return Err("User data format error".to_string());
    }

    use protobuf::CodedInputStream;
    use laoflchdb_engines::Message;
    let mut input_id = CodedInputStream::from_bytes(&row.data[0]);
    let id_field = laoflchdb_engines::Field::parse_from(&mut input_id)
        .map_err(|_| "Failed to parse user ID")?;

    let user_id = match id_field.value {
        Some(laoflchdb_engines::field::field::Value::IntegerValue(i)) => i.value,
        _ => return Err("User ID format error".to_string()),
    };

    let mut input_hash = CodedInputStream::from_bytes(&row.data[1]);
    let hash_field = laoflchdb_engines::Field::parse_from(&mut input_hash)
        .map_err(|_| "Failed to parse password hash")?;

    let stored_hash = match hash_field.value {
        Some(laoflchdb_engines::field::field::Value::StringValue(s)) => s.value,
        _ => return Err("Password hash format error".to_string()),
    };

    let input_hash = hash_password(password);
    if input_hash != stored_hash {
        return Err("Invalid username or password".to_string());
    }

    Ok(user_id)
}

#[derive(Deserialize, Debug)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Deserialize, Debug)]
pub struct LogoutRequest {
    pub token: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub success: bool,
    pub message: String,
    pub token: Option<String>,
    pub user_id: Option<i64>,
    pub username: Option<String>,
}

// 请求和响应结构
#[derive(Deserialize, Debug)]
pub struct GetQuery {
    pub schema: Option<String>,
    pub table: String,
    pub key: String,
}

#[derive(Deserialize, Debug)]
pub struct SqlQueryBody {
    pub schema: Option<String>,
    pub sql: String,
}

#[derive(Serialize)]
pub struct SqlQueryResponse {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
}

#[derive(Deserialize, Debug)]
pub struct PutBody {
    pub schema: Option<String>,
    pub table: String,
    pub key: String,
    pub value: String,
}

#[derive(Deserialize, Debug)]
pub struct DeleteKvBody {
    pub schema: Option<String>,
    pub table: String,
    pub key: String,
}

#[derive(Deserialize, Debug)]
pub struct CreateTableBody {
    pub schema: Option<String>,
    pub table_name: String,
    pub comment: Option<String>,
    pub columns: Vec<ColumnDefinition>,
}

#[derive(Deserialize, Debug)]
pub struct ColumnDefinition {
    pub name: String,
    pub column_type: String,
    pub comment: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct DropTableBody {
    pub schema: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct AddRowBody {
    pub schema: Option<String>,
    pub row: RestRow,
}

#[derive(Deserialize, Debug)]
pub struct UpdateRowBody {
    pub schema: Option<String>,
    pub row: RestRow,
}

#[derive(Deserialize, Debug, Serialize)]
pub struct RestRow {
    pub row_type: i32,
    pub version: u32,
    pub data: Vec<String>, // 每个字节数组编码为 hex 字符串
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
pub struct ColumnMetaResponse {
    pub table_id: u64,
    pub column_id: u64,
    pub column_name: String,
    pub column_type: i32,
}

#[derive(Serialize)]
pub struct GetResponse {
    pub value: Option<Vec<u8>>,
}

#[derive(Serialize)]
pub struct AddRowResponse {
    pub row_id: u64,
}

#[derive(Serialize)]
pub struct MetaJsonResponse {
    pub json: String,
}

// 辅助函数
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

fn encode_hex(data: &[u8]) -> String {
    data.iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

fn convert_rest_row_to_db_row(rest_row: &RestRow) -> Result<Row, String> {
    Ok(Row {
        row_type: EnumOrUnknown::new(RowType::from_i32(rest_row.row_type).unwrap_or(RowType::ROW_TYPE_NORMAL)),
        version: rest_row.version,
        data: rest_row.data.iter()
            .map(|s| decode_hex(s))
            .collect::<Result<Vec<_>, String>>()?,
        special_fields: SpecialFields::default(),
    })
}

fn encode_field(f: &laoflchdb_engines::Field) -> Vec<u8> {
    use protobuf::CodedOutputStream;
    use laoflchdb_engines::Message;
    let mut buf = Vec::new();
    {
        let mut os = CodedOutputStream::vec(&mut buf);
        let f_clone = f.clone();
        f_clone.compute_size();
        f_clone.write_to_with_cached_sizes(&mut os).unwrap_or_default();
        os.flush().unwrap_or_default();
    }
    buf
}

fn convert_rest_row_to_db_row_with_columns(rest_row: &RestRow, columns: &[ColumnMeta]) -> Result<Row, String> {
    use laoflchdb_engines::field::field::Value;
    use laoflchdb_engines::field::{String, Integer, Float, Bytes};
    
    let mut data = Vec::new();
    
    for (idx, value) in rest_row.data.iter().enumerate() {
        let column = columns.get(idx);
        let col_type = column.map(|c| c.column_type.enum_value_or_default()).unwrap_or(ColumnType::COLUMN_TYPE_STRING);
        
        let field = match col_type {
            ColumnType::COLUMN_TYPE_INT64 => {
                let val: i64 = value.parse().map_err(|e| format!("Failed to parse int64: {}", e))?;
                laoflchdb_engines::Field {
                    value: Some(Value::IntegerValue(Integer {
                        value: val,
                        special_fields: SpecialFields::default(),
                    })),
                    special_fields: SpecialFields::default(),
                }
            }
            ColumnType::COLUMN_TYPE_FLOAT => {
                let val: f64 = value.parse().map_err(|e| format!("Failed to parse float: {}", e))?;
                laoflchdb_engines::Field {
                    value: Some(Value::FloatValue(Float {
                        value: val,
                        special_fields: SpecialFields::default(),
                    })),
                    special_fields: SpecialFields::default(),
                }
            }
            ColumnType::COLUMN_TYPE_STRING => {
                laoflchdb_engines::Field {
                    value: Some(Value::StringValue(String {
                        value: value.clone(),
                        special_fields: SpecialFields::default(),
                    })),
                    special_fields: SpecialFields::default(),
                }
            }
            ColumnType::COLUMN_TYPE_BYTES => {
                let bytes = decode_hex(value)?;
                laoflchdb_engines::Field {
                    value: Some(Value::BytesValue(Bytes {
                        value: bytes,
                        special_fields: SpecialFields::default(),
                    })),
                    special_fields: SpecialFields::default(),
                }
            }
            _ => {
                laoflchdb_engines::Field {
                    value: Some(Value::StringValue(String {
                        value: value.clone(),
                        special_fields: SpecialFields::default(),
                    })),
                    special_fields: SpecialFields::default(),
                }
            }
        };
        
        data.push(encode_field(&field));
    }
    
    Ok(Row {
        row_type: EnumOrUnknown::new(RowType::from_i32(rest_row.row_type).unwrap_or(RowType::ROW_TYPE_NORMAL)),
        version: rest_row.version,
        data,
        special_fields: SpecialFields::default(),
    })
}

fn convert_db_row_to_rest_row(db_row: &Row) -> RestRow {
    RestRow {
        row_type: db_row.row_type.value(),
        version: db_row.version,
        data: db_row.data.iter()
            .map(|d| encode_hex(d))
            .collect(),
    }
}

fn convert_column_meta_to_rest(meta: &ColumnMeta) -> ColumnMetaResponse {
    ColumnMetaResponse {
        table_id: meta.table_id,
        column_id: meta.column_id,
        column_name: meta.column_name.clone(),
        column_type: meta.column_type.value(),
    }
}

async fn login_handler(
    State((service, _, _, token_manager)): State<SharedState>,
    Json(body): Json<LoginRequest>,
) -> Json<ApiResponse<LoginResponse>> {
    match verify_user(&service, &body.username, &body.password).await {
        Ok(user_id) => {
            let token = token_manager.generate_token(user_id, body.username.clone()).await;
            Json(ApiResponse::success(LoginResponse {
                success: true,
                message: "Login successful".to_string(),
                token: Some(token),
                user_id: Some(user_id),
                username: Some(body.username),
            }))
        }
        Err(msg) => {
            let msg_clone = msg.clone();
            Json(ApiResponse {
                success: false,
                message: msg,
                data: Some(LoginResponse {
                    success: false,
                    message: msg_clone,
                    token: None,
                    user_id: None,
                    username: None,
                }),
            })
        }
    }
}

async fn logout_handler(
    State((_, _, _, token_manager)): State<SharedState>,
    Json(body): Json<LogoutRequest>,
) -> Json<ApiResponse<&'static str>> {
    if token_manager.revoke_token(&body.token).await {
        Json(ApiResponse::success("Logout successful"))
    } else {
        Json(ApiResponse::error("Token not found or already expired".to_string()))
    }
}

// 处理函数
async fn health_handler() -> Json<ApiResponse<&'static str>> {
    Json(ApiResponse::success("OK"))
}

async fn get_handler(
    State((service, checker, service_id, _)): State<SharedState>,
    Query(query): Query<GetQuery>,
) -> Result<Json<ApiResponse<GetResponse>>, ApiError> {
    
    let schema = query.schema.unwrap_or_else(|| "sys".to_string());
    let context = PermissionContext {
        schema: schema.clone(),
        table: Some(query.table.clone()),
        action: PermissionAction::Get,
    };
    if !checker.check_permission(&service_id, &context).allowed {
        return Err(ApiError { message: "Permission denied".to_string() });
    }
    
    let key = match decode_hex(&query.key) {
        Ok(k) => k,
        Err(e) => return Ok(Json(ApiResponse::error(e))),
    };
    
    match service.get(&schema, &query.table, &key).await {
        Ok(value) => Ok(Json(ApiResponse::success(GetResponse { value }))),
        Err(e) => Ok(Json(ApiResponse::error(e.to_string()))),
    }
}

async fn put_handler(
    State((service, checker, service_id, _)): State<SharedState>,
    Json(body): Json<PutBody>,
) -> Result<Json<ApiResponse<&'static str>>, ApiError> {
    
    let schema = body.schema.unwrap_or_else(|| "sys".to_string());
    let context = PermissionContext {
        schema: schema.clone(),
        table: Some(body.table.clone()),
        action: PermissionAction::Put,
    };
    if !checker.check_permission(&service_id, &context).allowed {
        return Err(ApiError { message: "Permission denied".to_string() });
    }
    
    let key = match decode_hex(&body.key) {
        Ok(k) => k,
        Err(e) => return Ok(Json(ApiResponse::error(e))),
    };
    let value = match decode_hex(&body.value) {
        Ok(v) => v,
        Err(e) => return Ok(Json(ApiResponse::error(e))),
    };
    
    match service.put(&schema, &body.table, &key, &value).await {
        Ok(()) => Ok(Json(ApiResponse::success("OK"))),
        Err(e) => Ok(Json(ApiResponse::error(e.to_string()))),
    }
}

async fn delete_kv_handler(
    State((service, checker, service_id, _)): State<SharedState>,
    Json(body): Json<DeleteKvBody>,
) -> Result<Json<ApiResponse<&'static str>>, ApiError> {
    
    let schema = body.schema.unwrap_or_else(|| "sys".to_string());
    let context = PermissionContext {
        schema: schema.clone(),
        table: Some(body.table.clone()),
        action: PermissionAction::Delete,
    };
    if !checker.check_permission(&service_id, &context).allowed {
        return Err(ApiError { message: "Permission denied".to_string() });
    }
    
    let key = match decode_hex(&body.key) {
        Ok(k) => k,
        Err(e) => return Ok(Json(ApiResponse::error(e))),
    };
    
    match service.delete(&schema, &body.table, &key).await {
        Ok(()) => Ok(Json(ApiResponse::success("OK"))),
        Err(e) => Ok(Json(ApiResponse::error(e.to_string()))),
    }
}

async fn create_table_handler(
    State((service, checker, service_id, _)): State<SharedState>,
    Json(body): Json<CreateTableBody>,
) -> Result<Json<ApiResponse<CreateTableResponse>>, ApiError> {
    
    let schema = body.schema.unwrap_or_else(|| "sys".to_string());
    let context = PermissionContext {
        schema: schema.clone(),
        table: Some(body.table_name.clone()),
        action: PermissionAction::CreateTable,
    };
    if !checker.check_permission(&service_id, &context).allowed {
        return Err(ApiError { message: "Permission denied".to_string() });
    }
    
    let table_comment = body.comment.as_deref();
    
    let columns: Vec<(u32, &str, ColumnType, Option<&str>)> = body.columns
        .iter()
        .enumerate()
        .map(|(idx, col)| {
            let ct = match col.column_type.to_uppercase().as_str() {
                "STRING" => ColumnType::COLUMN_TYPE_STRING,
                "INT64" | "INT" => ColumnType::COLUMN_TYPE_INT64,
                "BYTES" | "BINARY" => ColumnType::COLUMN_TYPE_BYTES,
                "FLOAT" | "DOUBLE" => ColumnType::COLUMN_TYPE_FLOAT,
                "LIST" => ColumnType::COLUMN_TYPE_LIST,
                "IMAGE" => ColumnType::COLUMN_TYPE_IMAGE,
                _ => ColumnType::COLUMN_TYPE_STRING,
            };
            (idx as u32, col.name.as_str(), ct, col.comment.as_deref())
        })
        .collect();
    
    match service.create_table(&schema, &body.table_name, table_comment, &columns).await {
        Ok(table_id) => Ok(Json(ApiResponse::success(CreateTableResponse { table_id }))),
        Err(e) => Ok(Json(ApiResponse::error(e.to_string()))),
    }
}

async fn drop_table_handler(
    State((service, checker, service_id, _)): State<SharedState>,
    Path((schema, table)): Path<(String, String)>,
) -> Result<Json<ApiResponse<&'static str>>, ApiError> {
    
    let context = PermissionContext {
        schema: schema.clone(),
        table: Some(table.clone()),
        action: PermissionAction::DropTable,
    };
    if !checker.check_permission(&service_id, &context).allowed {
        return Err(ApiError { message: "Permission denied".to_string() });
    }
    
    match service.drop_table(&schema, &table).await {
        Ok(()) => Ok(Json(ApiResponse::success("OK"))),
        Err(e) => Ok(Json(ApiResponse::error(e.to_string()))),
    }
}

async fn list_tables_handler(
    State((service, checker, service_id, _)): State<SharedState>,
    Path(schema): Path<String>,
) -> Result<Json<ApiResponse<Vec<String>>>, ApiError> {
    
    let context = PermissionContext {
        schema: schema.clone(),
        table: None,
        action: PermissionAction::ListTables,
    };
    if !checker.check_permission(&service_id, &context).allowed {
        return Err(ApiError { message: "Permission denied".to_string() });
    }
    
    match service.list_tables(&schema).await {
        Ok(tables) => Ok(Json(ApiResponse::success(tables))),
        Err(e) => Ok(Json(ApiResponse::error(e.to_string()))),
    }
}

async fn list_table_cols_handler(
    State((service, checker, service_id, _)): State<SharedState>,
    Path((schema, table)): Path<(String, String)>,
) -> Result<Json<ApiResponse<Vec<ColumnMetaResponse>>>, ApiError> {
    
    let context = PermissionContext {
        schema: schema.clone(),
        table: Some(table.clone()),
        action: PermissionAction::ListTableCols,
    };
    if !checker.check_permission(&service_id, &context).allowed {
        return Err(ApiError { message: "Permission denied".to_string() });
    }
    
    match service.list_table_cols(&schema, &table).await {
        Ok(columns) => {
            let responses: Vec<_> = columns.iter()
                .map(convert_column_meta_to_rest)
                .collect();
            Ok(Json(ApiResponse::success(responses)))
        }
        Err(e) => Ok(Json(ApiResponse::error(e.to_string()))),
    }
}

async fn get_table_meta_handler(
    State((service, checker, service_id, _)): State<SharedState>,
    Path((schema, table)): Path<(String, String)>,
) -> Result<Json<ApiResponse<TableMetaResponse>>, ApiError> {
    
    let context = PermissionContext {
        schema: schema.clone(),
        table: Some(table.clone()),
        action: PermissionAction::GetTableMeta,
    };
    if !checker.check_permission(&service_id, &context).allowed {
        return Err(ApiError { message: "Permission denied".to_string() });
    }
    
    match service.get_table_meta(&schema, &table).await {
        Ok(Some(meta)) => {
            let response = TableMetaResponse {
                table_id: meta.table_id,
                table_name: meta.table_name,
                column_count: meta.column_count,
            };
            Ok(Json(ApiResponse::success(response)))
        }
        Ok(None) => Ok(Json(ApiResponse::error("Table not found".to_string()))),
        Err(e) => Ok(Json(ApiResponse::error(e.to_string()))),
    }
}

async fn add_row_handler(
    State((service, checker, service_id, _)): State<SharedState>,
    Path((schema, table)): Path<(String, String)>,
    Json(body): Json<AddRowBody>,
) -> Result<Json<ApiResponse<AddRowResponse>>, ApiError> {
    
    let context = PermissionContext {
        schema: schema.clone(),
        table: Some(table.clone()),
        action: PermissionAction::AddRow,
    };
    if !checker.check_permission(&service_id, &context).allowed {
        return Err(ApiError { message: "Permission denied".to_string() });
    }
    
    let columns = match service.list_table_cols(&schema, &table).await {
        Ok(cols) => cols,
        Err(e) => return Ok(Json(ApiResponse::error(format!("Failed to get columns: {}", e)))),
    };
    
    let db_row = match convert_rest_row_to_db_row_with_columns(&body.row, &columns) {
        Ok(row) => row,
        Err(e) => return Ok(Json(ApiResponse::error(e))),
    };
    
    match service.add_row(&schema, &table, &db_row).await {
        Ok(row_id) => Ok(Json(ApiResponse::success(AddRowResponse { row_id }))),
        Err(e) => Ok(Json(ApiResponse::error(e.to_string()))),
    }
}

async fn get_row_handler(
    State((service, checker, service_id, _)): State<SharedState>,
    Path((schema, table, row_id)): Path<(String, String, u64)>,
) -> Result<Json<ApiResponse<RestRow>>, ApiError> {
    
    let context = PermissionContext {
        schema: schema.clone(),
        table: Some(table.clone()),
        action: PermissionAction::GetRow,
    };
    if !checker.check_permission(&service_id, &context).allowed {
        return Err(ApiError { message: "Permission denied".to_string() });
    }
    
    match service.get_row(&schema, &table, row_id).await {
        Ok(Some(row)) => Ok(Json(ApiResponse::success(convert_db_row_to_rest_row(&row)))),
        Ok(None) => Ok(Json(ApiResponse::error("Row not found".to_string()))),
        Err(e) => Ok(Json(ApiResponse::error(e.to_string()))),
    }
}

async fn delete_row_handler(
    State((service, checker, service_id, _)): State<SharedState>,
    Path((schema, table, row_id)): Path<(String, String, u64)>,
) -> Result<Json<ApiResponse<&'static str>>, ApiError> {
    
    let context = PermissionContext {
        schema: schema.clone(),
        table: Some(table.clone()),
        action: PermissionAction::DeleteRow,
    };
    if !checker.check_permission(&service_id, &context).allowed {
        return Err(ApiError { message: "Permission denied".to_string() });
    }
    
    match service.delete_row(&schema, &table, row_id).await {
        Ok(()) => Ok(Json(ApiResponse::success("OK"))),
        Err(e) => Ok(Json(ApiResponse::error(e.to_string()))),
    }
}

async fn update_row_handler(
    State((service, checker, service_id, _)): State<SharedState>,
    Path((schema, table, row_id)): Path<(String, String, u64)>,
    Json(body): Json<UpdateRowBody>,
) -> Result<Json<ApiResponse<&'static str>>, ApiError> {
    
    let context = PermissionContext {
        schema: schema.clone(),
        table: Some(table.clone()),
        action: PermissionAction::UpdateRow,
    };
    if !checker.check_permission(&service_id, &context).allowed {
        return Err(ApiError { message: "Permission denied".to_string() });
    }
    
    let db_row = match convert_rest_row_to_db_row(&body.row) {
        Ok(row) => row,
        Err(e) => return Ok(Json(ApiResponse::error(e))),
    };
    
    match service.update_row(&schema, &table, row_id, &db_row).await {
        Ok(()) => Ok(Json(ApiResponse::success("OK"))),
        Err(e) => Ok(Json(ApiResponse::error(e.to_string()))),
    }
}

async fn get_all_meta_handler(
    State((service, checker, service_id, _)): State<SharedState>,
    Path(schema): Path<String>,
) -> Result<Json<ApiResponse<MetaJsonResponse>>, ApiError> {
    
    let context = PermissionContext {
        schema: schema.clone(),
        table: None,
        action: PermissionAction::GetAllMeta,
    };
    if !checker.check_permission(&service_id, &context).allowed {
        return Err(ApiError { message: "Permission denied".to_string() });
    }
    
    match service.get_all_meta(&schema).await {
        Ok(json) => Ok(Json(ApiResponse::success(MetaJsonResponse { json }))),
        Err(e) => Ok(Json(ApiResponse::error(e.to_string()))),
    }
}

async fn get_schema_info_handler(
    State((service, checker, service_id, _)): State<SharedState>,
    Path(schema): Path<String>,
) -> Result<Json<ApiResponse<MetaJsonResponse>>, ApiError> {
    
    let context = PermissionContext {
        schema: schema.clone(),
        table: None,
        action: PermissionAction::GetSchemaInfo,
    };
    if !checker.check_permission(&service_id, &context).allowed {
        return Err(ApiError { message: "Permission denied".to_string() });
    }
    
    match service.get_schema_info(&schema).await {
        Ok(json) => Ok(Json(ApiResponse::success(MetaJsonResponse { json }))),
        Err(e) => Ok(Json(ApiResponse::error(e.to_string()))),
    }
}

async fn sql_query_handler(
    State((service, checker, service_id, _)): State<SharedState>,
    Json(body): Json<SqlQueryBody>,
) -> Result<Json<ApiResponse<SqlQueryResponse>>, ApiError> {
    
    let schema = body.schema.unwrap_or_else(|| "sys".to_string());
    let context = PermissionContext {
        schema: schema.clone(),
        table: None,
        action: PermissionAction::Query,
    };
    if !checker.check_permission(&service_id, &context).allowed {
        return Err(ApiError { message: "Permission denied".to_string() });
    }
    
    match service.sql_query(&schema, &body.sql).await {
        Ok(result) => {
            let columns: Vec<String> = if !result.columns.is_empty() {
                result.columns.clone()
            } else if !result.rows.is_empty() {
                if let Some(first_row) = result.rows.first() {
                    if first_row.row.is_some() {
                        let row = first_row.row.get_or_default();
                        row.data.iter().enumerate()
                            .map(|(idx, _)| format!("col_{}", idx))
                            .collect()
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };
            
            let mut rows: Vec<Vec<serde_json::Value>> = Vec::new();
            for qr in &result.rows {
                if qr.row.is_some() {
                    let row = qr.row.get_or_default();
                    let row_values: Vec<serde_json::Value> = row.data.iter()
                        .map(|d| {
                            use protobuf::CodedInputStream;
                            use laoflchdb_engines::Message;
                            let mut input = CodedInputStream::from_bytes(d);
                            if let Ok(field) = laoflchdb_engines::Field::parse_from(&mut input) {
                                use laoflchdb_engines::field::field::Value;
                                match field.value {
                                    Some(Value::StringValue(s)) => serde_json::Value::String(s.value),
                                    Some(Value::IntegerValue(i)) => serde_json::Value::Number(serde_json::Number::from(i.value)),
                                    Some(Value::FloatValue(f)) => serde_json::Value::Number(serde_json::Number::from_f64(f.value).unwrap_or(serde_json::Number::from(0))),
                                    Some(Value::BytesValue(b)) => serde_json::Value::String(String::from_utf8_lossy(&b.value).to_string()),
                                    _ => serde_json::Value::Null,
                                }
                            } else {
                                serde_json::Value::String(String::from_utf8_lossy(d).to_string())
                            }
                        })
                        .collect();
                    rows.push(row_values);
                }
            }
            
            Ok(Json(ApiResponse::success(SqlQueryResponse { columns, rows })))
        }
        Err(e) => Ok(Json(ApiResponse::error(e.to_string()))),
    }
}

// ==================== Index 相关结构和处理器 ====================

type IndexSharedState = (Arc<dyn IndexService>, Arc<PermissionChecker>, String, Arc<TokenManager>);

async fn index_auth_middleware(
    State((_, _, _, token_manager)): State<IndexSharedState>,
    req: axum::http::Request<axum::body::Body>,
    next: middleware::Next,
) -> Result<axum::response::Response, ApiError> {
    let auth_header = req.headers().get(axum::http::header::AUTHORIZATION);
    
    if let Some(header) = auth_header {
        let header_str = header.to_str().map_err(|_| ApiError { 
            message: "Invalid authorization header".to_string() 
        })?;
        
        if header_str.starts_with("Bearer ") {
            let token = &header_str[7..];
            if token_manager.validate_token(token).await.is_some() {
                return Ok(next.run(req).await);
            }
        }
    }
    
    Err(ApiError { 
        message: "Unauthorized: Invalid or missing token".to_string() 
    })
}

#[derive(Deserialize, Debug)]
pub struct CreateIndexRequest {
    pub index_name: String,
    pub fields: Vec<IndexFieldDefinition>,
}

#[derive(Deserialize, Debug)]
pub struct IndexFieldDefinition {
    pub name: String,
    pub field_type: String,
    pub comment: Option<String>,
}

#[derive(Serialize)]
pub struct CreateIndexResponse {
    pub index_id: u64,
}

#[derive(Serialize)]
pub struct IndexListResponse {
    pub indices: Vec<String>,
}

#[derive(Serialize)]
pub struct IndexFieldResponse {
    pub column_id: u64,
    pub column_name: String,
    pub column_type: i32,
}

#[derive(Serialize)]
pub struct IndexMetaResponse {
    pub table_id: u64,
    pub table_name: String,
    pub column_count: u32,
    pub comment: String,
}

#[derive(Deserialize, Debug)]
pub struct AddDocumentRequest {
    pub doc_id: String,
    pub fields: std::collections::HashMap<String, String>,
}

#[derive(Serialize)]
pub struct AddDocumentResponse {
    pub doc_id: u64,
}

#[derive(Deserialize, Debug)]
pub struct SearchQuery {
    pub q: String,
    pub limit: Option<usize>,
}

#[derive(Serialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResultResponse>,
}

#[derive(Serialize)]
pub struct SearchResultResponse {
    pub doc_id: String,
    pub score: f32,
    pub fields: std::collections::HashMap<String, String>,
}

#[derive(Deserialize, Debug)]
pub struct MultiFieldSearchRequest {
    pub field_queries: std::collections::HashMap<String, String>,
    pub limit: Option<usize>,
}

#[derive(Serialize)]
pub struct IndexStatsResponse {
    pub total_indices: usize,
    pub index_names: Vec<String>,
}

async fn create_index_handler(
    State((index_service, _, _, _)): State<IndexSharedState>,
    Json(body): Json<CreateIndexRequest>,
) -> Result<Json<ApiResponse<CreateIndexResponse>>, ApiError> {    
    let columns: Vec<(u32, &str, ColumnType, Option<&str>)> = body.fields
        .iter()
        .enumerate()
        .map(|(idx, field)| {
            let ct = match field.field_type.to_uppercase().as_str() {
                "STRING" => ColumnType::COLUMN_TYPE_STRING,
                "INT64" | "INT" => ColumnType::COLUMN_TYPE_INT64,
                "BYTES" | "BINARY" => ColumnType::COLUMN_TYPE_BYTES,
                "FLOAT" | "DOUBLE" => ColumnType::COLUMN_TYPE_FLOAT,
                _ => ColumnType::COLUMN_TYPE_STRING,
            };
            (idx as u32, field.name.as_str(), ct, field.comment.as_deref())
        })
        .collect();
    println!("{:?}", body);
    match index_service.create_index(&body.index_name, &columns).await {
        Ok(index_id) => Ok(Json(ApiResponse::success(CreateIndexResponse { index_id }))),
        Err(e) => Ok(Json(ApiResponse::error(e.to_string()))),
    }
}

async fn list_indices_handler(
    State((index_service, _, _, _)): State<IndexSharedState>,
) -> Result<Json<ApiResponse<IndexListResponse>>, ApiError> {    
    match index_service.list_indices().await {
        Ok(indices) => Ok(Json(ApiResponse::success(IndexListResponse { indices }))),
        Err(e) => Ok(Json(ApiResponse::error(e.to_string()))),
    }
}

async fn drop_index_handler(
    State((index_service, _, _, _)): State<IndexSharedState>,
    Path(index_name): Path<String>,
) -> Result<Json<ApiResponse<&'static str>>, ApiError> {    
    match index_service.drop_index(&index_name).await {
        Ok(()) => Ok(Json(ApiResponse::success("OK"))),
        Err(e) => Ok(Json(ApiResponse::error(e.to_string()))),
    }
}

async fn get_index_fields_handler(
    State((index_service, _, _, _)): State<IndexSharedState>,
    Path(index_name): Path<String>,
) -> Result<Json<ApiResponse<Vec<IndexFieldResponse>>>, ApiError> {    
    match index_service.get_index_fields(&index_name).await {
        Ok(fields) => {
            let responses: Vec<_> = fields.iter()
                .map(|f| IndexFieldResponse {
                    column_id: f.column_id,
                    column_name: f.column_name.clone(),
                    column_type: f.column_type.value(),
                })
                .collect();
            Ok(Json(ApiResponse::success(responses)))
        }
        Err(e) => Ok(Json(ApiResponse::error(e.to_string()))),
    }
}

async fn get_index_meta_handler(
    State((index_service, _, _, _)): State<IndexSharedState>,
    Path(index_name): Path<String>,
) -> Result<Json<ApiResponse<IndexMetaResponse>>, ApiError> {    
    match index_service.get_index_meta(&index_name).await {
        Ok(Some(meta)) => {
            let response = IndexMetaResponse {
                table_id: meta.table_id,
                table_name: meta.table_name,
                column_count: meta.column_count,
                comment: meta.comment,
            };
            Ok(Json(ApiResponse::success(response)))
        }
        Ok(None) => Ok(Json(ApiResponse::error("Index not found".to_string()))),
        Err(e) => Ok(Json(ApiResponse::error(e.to_string()))),
    }
}

async fn add_document_handler(
    State((index_service, _, _, _)): State<IndexSharedState>,
    Path(index_name): Path<String>,
    Json(body): Json<AddDocumentRequest>,
) -> Result<Json<ApiResponse<AddDocumentResponse>>, ApiError> {    
    match index_service.add_document(&index_name, &body.doc_id, body.fields).await {
        Ok(doc_id) => Ok(Json(ApiResponse::success(AddDocumentResponse { doc_id }))),
        Err(e) => Ok(Json(ApiResponse::error(e.to_string()))),
    }
}

async fn delete_document_handler(
    State((index_service, _, _, _)): State<IndexSharedState>,
    Path((index_name, doc_id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<&'static str>>, ApiError> {    
    match index_service.delete_document(&index_name, &doc_id).await {
        Ok(()) => Ok(Json(ApiResponse::success("OK"))),
        Err(e) => Ok(Json(ApiResponse::error(e.to_string()))),
    }
}

async fn search_handler(
    State((index_service, _, _, _)): State<IndexSharedState>,
    Path(index_name): Path<String>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<ApiResponse<SearchResponse>>, ApiError> {    
    match index_service.search(&index_name, &query.q, query.limit).await {
        Ok(results) => {
            let responses: Vec<_> = results.iter()
                .map(|r| SearchResultResponse {
                    doc_id: r.doc_id.clone(),
                    score: r.score,
                    fields: r.fields.clone(),
                })
                .collect();
            Ok(Json(ApiResponse::success(SearchResponse { results: responses })))
        }
        Err(e) => Ok(Json(ApiResponse::error(e.to_string()))),
    }
}

async fn search_multi_field_handler(
    State((index_service, _, _, _)): State<IndexSharedState>,
    Path(index_name): Path<String>,
    Json(body): Json<MultiFieldSearchRequest>,
) -> Result<Json<ApiResponse<SearchResponse>>, ApiError> {    
    match index_service.search_multi_field(&index_name, body.field_queries, body.limit).await {
        Ok(results) => {
            let responses: Vec<_> = results.iter()
                .map(|r| SearchResultResponse {
                    doc_id: r.doc_id.clone(),
                    score: r.score,
                    fields: r.fields.clone(),
                })
                .collect();
            Ok(Json(ApiResponse::success(SearchResponse { results: responses })))
        }
        Err(e) => Ok(Json(ApiResponse::error(e.to_string()))),
    }
}

async fn get_index_stats_handler(
    State((index_service, _, _, _)): State<IndexSharedState>,
) -> Result<Json<ApiResponse<IndexStatsResponse>>, ApiError> {
    match index_service.get_stats().await {
        Ok(stats) => {
            let response = IndexStatsResponse {
                total_indices: stats.total_indices,
                index_names: stats.index_names,
            };
            Ok(Json(ApiResponse::success(response)))
        }
        Err(e) => Ok(Json(ApiResponse::error(e.to_string()))),
    }
}
