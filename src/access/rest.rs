use crate::service::DatabaseService;
use crate::access::{PermissionChecker, PermissionContext};
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
    http::StatusCode,
    response::IntoResponse,
};

#[derive(Clone)]
pub struct RestService {
    service: Arc<dyn DatabaseService>,
    permission_checker: Arc<PermissionChecker>,
    service_id: String,
}

impl RestService {
    pub fn new(service: Arc<dyn DatabaseService>) -> Self {
        Self {
            service,
            permission_checker: Arc::new(PermissionChecker::new(true)),
            service_id: "default".to_string(),
        }
    }

    pub fn with_permissions(service: Arc<dyn DatabaseService>, permission_checker: Arc<PermissionChecker>, service_id: String) -> Self {
        Self {
            service,
            permission_checker,
            service_id,
        }
    }

    fn check_permission(&self, schema: &str, table: Option<&str>, action: PermissionAction) -> Result<(), StatusCode> {
        let context = PermissionContext {
            schema: schema.to_string(),
            table: table.map(String::from),
            action,
        };
        let result = self.permission_checker.check_permission(&self.service_id, &context);
        if !result.allowed {
            log::warn!(
                "Permission denied for REST service '{}' on action '{}': {}",
                self.service_id,
                context.action,
                result.reason
            );
            Err(StatusCode::FORBIDDEN)
        } else {
            Ok(())
        }
    }

    pub fn router(&self) -> Router {
        Router::new()
            .route("/health", get(health_handler))
            // KV 操作
            .route("/api/v1/get", get(get_handler))
            .route("/api/v1/put", post(put_handler))
            .route("/api/v1/delete", post(delete_kv_handler))
            // 表管理
            .route("/api/v1/tables", post(create_table_handler))
            .route("/api/v1/tables/:table", delete(drop_table_handler))
            .route("/api/v1/schemas/:schema/tables", get(list_tables_handler))
            .route("/api/v1/schemas/:schema/tables/:table/columns", get(list_table_cols_handler))
            .route("/api/v1/schemas/:schema/tables/:table", get(get_table_meta_handler))
            // 行操作
            .route("/api/v1/schemas/:schema/tables/:table/rows", post(add_row_handler))
            .route("/api/v1/schemas/:schema/tables/:table/rows/:row_id", get(get_row_handler))
            .route("/api/v1/schemas/:schema/tables/:table/rows/:row_id", delete(delete_row_handler))
            .route("/api/v1/schemas/:schema/tables/:table/rows/:row_id", put(update_row_handler))
            // 元数据查询
            .route("/api/v1/schemas/:schema/meta", get(get_all_meta_handler))
            .route("/api/v1/schemas/:schema/info", get(get_schema_info_handler))
            .with_state((self.service.clone(), self.permission_checker.clone(), self.service_id.clone()))
    }

    pub async fn start(&self, addr: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        log::info!("REST server listening on {}", addr);
        axum::serve(listener, self.router()).await?;
        Ok(())
    }
}

type SharedState = (Arc<dyn DatabaseService>, Arc<PermissionChecker>, String);

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

// 请求和响应结构
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
pub struct DeleteKvBody {
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

// 处理函数
async fn health_handler() -> Json<ApiResponse<&'static str>> {
    Json(ApiResponse::success("OK"))
}

async fn get_handler(
    State((service, checker, service_id)): State<SharedState>,
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
    State((service, checker, service_id)): State<SharedState>,
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
    State((service, checker, service_id)): State<SharedState>,
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
    State((service, checker, service_id)): State<SharedState>,
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
    
    let columns: Vec<(u32, &str, ColumnType)> = body.columns
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
            (idx as u32, col.name.as_str(), ct)
        })
        .collect();
    
    match service.create_table(&schema, &body.table_name, &columns).await {
        Ok(table_id) => Ok(Json(ApiResponse::success(CreateTableResponse { table_id }))),
        Err(e) => Ok(Json(ApiResponse::error(e.to_string()))),
    }
}

async fn drop_table_handler(
    State((service, checker, service_id)): State<SharedState>,
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
    State((service, checker, service_id)): State<SharedState>,
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
    State((service, checker, service_id)): State<SharedState>,
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
    State((service, checker, service_id)): State<SharedState>,
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
    State((service, checker, service_id)): State<SharedState>,
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
    
    let db_row = match convert_rest_row_to_db_row(&body.row) {
        Ok(row) => row,
        Err(e) => return Ok(Json(ApiResponse::error(e))),
    };
    
    match service.add_row(&schema, &table, &db_row).await {
        Ok(row_id) => Ok(Json(ApiResponse::success(AddRowResponse { row_id }))),
        Err(e) => Ok(Json(ApiResponse::error(e.to_string()))),
    }
}

async fn get_row_handler(
    State((service, checker, service_id)): State<SharedState>,
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
    State((service, checker, service_id)): State<SharedState>,
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
    State((service, checker, service_id)): State<SharedState>,
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
    State((service, checker, service_id)): State<SharedState>,
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
    State((service, checker, service_id)): State<SharedState>,
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
