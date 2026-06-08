use crate::service::DatabaseService;
use crate::pb::rpc::{
    laoflch_db_server::LaoflchDb,
    GetRequest, GetResponse,
    PutRequest, PutResponse,
    DeleteRequest, DeleteResponse,
    CreateTableRequest, CreateTableResponse,
    DropTableRequest, DropTableResponse,
    ListTablesRequest, ListTablesResponse,
    ListTableColsRequest, ListTableColsResponse,
    AddRowRequest, AddRowResponse,
    GetRowRequest, GetRowResponse,
    DeleteRowRequest, DeleteRowResponse,
    UpdateRowRequest, UpdateRowResponse,
    GetAllMetaRequest, GetAllMetaResponse,
    GetSchemaInfoRequest, GetSchemaInfoResponse,
    ListSchemasRequest, ListSchemasResponse,
    GetTableMetaRequest, GetTableMetaResponse,
    QueryRequest, QueryResponse,
    SqlQueryRequest, SqlQueryResponse,
    RefreshTablesRequest, RefreshTablesResponse,
    GetVersionRequest, GetVersionResponse,
    SqlQueryResultRow,
    SqlField,
    ColumnMeta as RpcColumnMeta,
    Row as RpcRow,
};
use crate::config::PermissionAction;
use protobuf::Enum;
use laoflchdb_engines::{ColumnMeta, Row, ColumnType, Query, QueryResult, QueryRow, SpecialFields};
use std::sync::Arc;
use tonic::{Request, Response, Status};

pub mod rest;
pub mod permission;
pub use rest::RestService;
pub use permission::{PermissionChecker, PermissionContext, PermissionCheckResult};

#[derive(Clone)]
pub struct GrpcService {
    service: Arc<dyn DatabaseService>,
    permission_checker: Option<Arc<PermissionChecker>>,
    service_id: String,
}

impl GrpcService {
    pub fn new(service: Arc<dyn DatabaseService>) -> Self {
        Self {
            service,
            permission_checker: None,
            service_id: "default".to_string(),
        }
    }

    pub fn with_permissions(
        service: Arc<dyn DatabaseService>,
        permission_checker: Arc<PermissionChecker>,
        service_id: String,
    ) -> Self {
        Self {
            service,
            permission_checker: Some(permission_checker),
            service_id,
        }
    }

    fn check_permission(&self, schema: &str, table: Option<&str>, action: PermissionAction) -> Result<(), Status> {
        if let Some(ref checker) = self.permission_checker {
            let context = PermissionContext {
                schema: schema.to_string(),
                table: table.map(String::from),
                action,
            };
            let action_name = context.action.to_string();
            let result = checker.check_permission(&self.service_id, &context);
            if !result.allowed {
                log::warn!(
                    "Permission denied for service '{}' on action '{}': {}",
                    self.service_id,
                    action_name,
                    result.reason
                );
                return Err(Status::permission_denied(result.reason));
            }
        }
        Ok(())
    }
}

pub struct AccessService {
    service: Arc<dyn DatabaseService>,
    permission_checker: Option<Arc<PermissionChecker>>,
}

impl AccessService {
    pub fn new(service: Arc<dyn DatabaseService>) -> Self {
        Self {
            service,
            permission_checker: None,
        }
    }

    pub fn with_permissions(service: Arc<dyn DatabaseService>, permission_checker: Arc<PermissionChecker>) -> Self {
        Self {
            service,
            permission_checker: Some(permission_checker),
        }
    }

    pub fn get_grpc_service(&self, service_id: Option<String>) -> GrpcService {
        let sid = service_id.unwrap_or_else(|| "default".to_string());
        if let Some(ref checker) = self.permission_checker {
            if let Some(perm) = checker.get_service_policy(&sid) {
                return GrpcService::with_permissions(
                    Arc::clone(&self.service),
                    Arc::clone(checker),
                    perm.service_id.clone(),
                );
            }
        }
        GrpcService::with_permissions(
            Arc::clone(&self.service),
            Arc::new(PermissionChecker::new(true)),
            sid,
        )
    }

    pub fn get_rest_service(&self, service_id: Option<String>) -> RestService {
        let sid = service_id.unwrap_or_else(|| "default".to_string());
        if let Some(ref checker) = self.permission_checker {
            if let Some(perm) = checker.get_service_policy(&sid) {
                return RestService::with_permissions(
                    Arc::clone(&self.service),
                    Arc::clone(checker),
                    perm.service_id.clone(),
                );
            }
        }
        RestService::with_permissions(
            Arc::clone(&self.service),
            Arc::new(PermissionChecker::new(true)),
            sid,
        )
    }

    pub fn get_service(&self) -> Arc<dyn DatabaseService> {
        Arc::clone(&self.service)
    }
}

fn convert_column_meta_to_rpc(meta: &ColumnMeta) -> RpcColumnMeta {
    RpcColumnMeta {
        table_id: meta.table_id,
        column_id: meta.column_id,
        column_name: meta.column_name.clone(),
        column_type: meta.column_type.value(),
    }
}

fn convert_row_from_rpc(rpc_row: RpcRow) -> Row {
    use laoflchdb_engines::{EnumOrUnknown, RowType};
    Row {
        row_type: EnumOrUnknown::new(RowType::from_i32(rpc_row.row_type).unwrap_or(RowType::ROW_TYPE_NORMAL)),
        version: rpc_row.version,
        data: rpc_row.data,
        special_fields: SpecialFields::default(),
    }
}

fn convert_row_to_rpc(row: &Row) -> RpcRow {
    RpcRow {
        row_type: row.row_type.value(),
        version: row.version,
        data: row.data.clone(),
    }
}

fn convert_query_from_rpc(req: &QueryRequest) -> Query {
    use laoflchdb_engines::{TableFilter, ColumnFilter, ColumnFilterCondition, FilterOperator, Field, EnumOrUnknown};
    use laoflchdb_engines::field::field::Value;
    use laoflchdb_engines::field::{String, Integer, Bytes, Float, List, Image};
    
    let table_filters = req.table_filters.iter().map(|tf| {
        let column_filters = tf.column_filters.iter().map(|cf| {
            let conditions = cf.conditions.iter().map(|cond| {
                let op = FilterOperator::from_i32(cond.op).unwrap_or(FilterOperator::FILTER_OPERATOR_UNSPECIFIED);
                
                let field_value = cond.value.as_ref().map(|f| {
                    let val = match f.value {
                        Some(ref v) => match v {
                            crate::pb::rpc::field::Value::StringValue(s) => Value::StringValue(String {
                                value: s.value.clone(),
                                special_fields: SpecialFields::default(),
                            }),
                            crate::pb::rpc::field::Value::IntegerValue(i) => Value::IntegerValue(Integer {
                                value: i.value,
                                special_fields: SpecialFields::default(),
                            }),
                            crate::pb::rpc::field::Value::BytesValue(b) => Value::BytesValue(Bytes {
                                value: b.value.clone(),
                                special_fields: SpecialFields::default(),
                            }),
                            crate::pb::rpc::field::Value::FloatValue(fv) => Value::FloatValue(Float {
                                value: fv.value,
                                special_fields: SpecialFields::default(),
                            }),
                            crate::pb::rpc::field::Value::ListValue(l) => Value::ListValue(List {
                                items: l.items.clone(),
                                special_fields: SpecialFields::default(),
                            }),
                            crate::pb::rpc::field::Value::ImageValue(img) => Value::ImageValue(Image {
                                data: img.data.clone(),
                                format: img.format.clone(),
                                special_fields: SpecialFields::default(),
                            }),
                        },
                        None => Value::StringValue(String { 
                            value: std::string::String::new(),
                            special_fields: SpecialFields::default(),
                        }),
                    };
                    
                    Field { value: Some(val), special_fields: SpecialFields::default() }
                });
                
                let values = cond.values.iter().map(|f| {
                    let val = match f.value {
                        Some(ref v) => match v {
                            crate::pb::rpc::field::Value::StringValue(s) => Value::StringValue(String {
                                value: s.value.clone(),
                                special_fields: SpecialFields::default(),
                            }),
                            crate::pb::rpc::field::Value::IntegerValue(i) => Value::IntegerValue(Integer {
                                value: i.value,
                                special_fields: SpecialFields::default(),
                            }),
                            crate::pb::rpc::field::Value::BytesValue(b) => Value::BytesValue(Bytes {
                                value: b.value.clone(),
                                special_fields: SpecialFields::default(),
                            }),
                            crate::pb::rpc::field::Value::FloatValue(fv) => Value::FloatValue(Float {
                                value: fv.value,
                                special_fields: SpecialFields::default(),
                            }),
                            crate::pb::rpc::field::Value::ListValue(l) => Value::ListValue(List {
                                items: l.items.clone(),
                                special_fields: SpecialFields::default(),
                            }),
                            crate::pb::rpc::field::Value::ImageValue(img) => Value::ImageValue(Image {
                                data: img.data.clone(),
                                format: img.format.clone(),
                                special_fields: SpecialFields::default(),
                            }),
                        },
                        None => Value::StringValue(String { 
                            value: std::string::String::new(),
                            special_fields: SpecialFields::default(),
                        }),
                    };
                    
                    Field { value: Some(val), special_fields: SpecialFields::default() }
                }).collect();
                
                ColumnFilterCondition {
                    op: EnumOrUnknown::new(op),
                    value: field_value.into(),
                    values,
                    special_fields: SpecialFields::default(),
                }
            }).collect();
            
            ColumnFilter {
                column_name: cf.column_name.clone(),
                conditions,
                special_fields: SpecialFields::default(),
            }
        }).collect();
        
        TableFilter {
            table_name: tf.table_name.clone(),
            column_filters,
            special_fields: SpecialFields::default(),
        }
    }).collect();
    
    Query {
        table_filters,
        limit: req.limit,
        offset: req.offset,
        projected_columns: req.projected_columns.clone(),
        special_fields: SpecialFields::default(),
    }
}

fn convert_query_row_to_rpc(qr: &QueryRow) -> crate::pb::rpc::QueryRow {
    crate::pb::rpc::QueryRow {
        table_name: qr.table_name.clone(),
        row_id: qr.row_id,
        row: qr.row.as_ref().map(|r| convert_row_to_rpc(r)),
    }
}

#[tonic::async_trait]
impl LaoflchDb for GrpcService {
    async fn get(&self, request: Request<GetRequest>) -> Result<Response<GetResponse>, Status> {
        let req = request.into_inner();
        let schema = if req.schema.is_empty() { "sys" } else { &req.schema };
        
        self.check_permission(schema, Some(&req.table), PermissionAction::Get)?;
        
        match self.service.get(schema, &req.table, &req.key).await {
            Ok(value) => Ok(Response::new(GetResponse {
                success: true,
                value: value.unwrap_or_default(),
                message: String::new(),
            })),
            Err(e) => Err(Status::internal(e.to_string())),
        }
    }

    async fn put(&self, request: Request<PutRequest>) -> Result<Response<PutResponse>, Status> {
        let req = request.into_inner();
        let schema = if req.schema.is_empty() { "sys" } else { &req.schema };
        
        self.check_permission(schema, Some(&req.table), PermissionAction::Put)?;
        
        match self.service.put(schema, &req.table, &req.key, &req.value).await {
            Ok(()) => Ok(Response::new(PutResponse { 
                success: true,
                message: String::new(),
            })),
            Err(e) => Err(Status::internal(e.to_string())),
        }
    }

    async fn delete(&self, request: Request<DeleteRequest>) -> Result<Response<DeleteResponse>, Status> {
        let req = request.into_inner();
        let schema = if req.schema.is_empty() { "sys" } else { &req.schema };
        
        self.check_permission(schema, Some(&req.table), PermissionAction::Delete)?;
        
        match self.service.delete(schema, &req.table, &req.key).await {
            Ok(()) => Ok(Response::new(DeleteResponse { 
                success: true,
                message: String::new(),
            })),
            Err(e) => Err(Status::internal(e.to_string())),
        }
    }

    async fn create_table(&self, request: Request<CreateTableRequest>) -> Result<Response<CreateTableResponse>, Status> {
        let req = request.into_inner();
        let schema = if req.schema.is_empty() { "sys" } else { &req.schema };
        let table_name = req.table_name.as_str();

        self.check_permission(schema, Some(table_name), PermissionAction::CreateTable)?;

        let columns: Vec<(u32, String, ColumnType)> = req.columns
            .into_iter()
            .enumerate()
            .map(|(idx, col)| {
                let ct = ColumnType::from_i32(col.column_type).unwrap_or(ColumnType::COLUMN_TYPE_STRING);
                (idx as u32, col.name, ct)
            })
            .collect();
        
        let columns_ref: Vec<(u32, &str, ColumnType)> = columns.iter()
            .map(|(id, name, col_type)| (*id, name.as_str(), *col_type))
            .collect();

        match self.service.create_table(schema, table_name, &columns_ref).await {
            Ok(table_id) => Ok(Response::new(CreateTableResponse {
                success: true,
                table_id,
                message: String::new(),
            })),
            Err(e) => Err(Status::internal(e.to_string())),
        }
    }

    async fn drop_table(&self, request: Request<DropTableRequest>) -> Result<Response<DropTableResponse>, Status> {
        let req = request.into_inner();
        let schema = if req.schema.is_empty() { "sys" } else { &req.schema };
        
        self.check_permission(schema, Some(&req.table_name), PermissionAction::DropTable)?;
        
        match self.service.drop_table(schema, &req.table_name).await {
            Ok(()) => Ok(Response::new(DropTableResponse {
                success: true,
                message: String::new(),
            })),
            Err(e) => Err(Status::internal(e.to_string())),
        }
    }

    async fn list_tables(&self, request: Request<ListTablesRequest>) -> Result<Response<ListTablesResponse>, Status> {
        let req = request.into_inner();
        let schema = if req.schema.is_empty() { "sys" } else { &req.schema };
        
        self.check_permission(schema, None, PermissionAction::ListTables)?;
        
        match self.service.list_tables(schema).await {
            Ok(tables) => Ok(Response::new(ListTablesResponse {
                success: true,
                tables,
                message: String::new(),
            })),
            Err(e) => Err(Status::internal(e.to_string())),
        }
    }

    async fn list_table_cols(&self, request: Request<ListTableColsRequest>) -> Result<Response<ListTableColsResponse>, Status> {
        let req = request.into_inner();
        let schema = if req.schema.is_empty() { "sys" } else { &req.schema };
        
        self.check_permission(schema, Some(&req.table_name), PermissionAction::ListTableCols)?;
        
        match self.service.list_table_cols(schema, &req.table_name).await {
            Ok(columns) => Ok(Response::new(ListTableColsResponse {
                success: true,
                columns: columns.iter().map(convert_column_meta_to_rpc).collect(),
                message: String::new(),
            })),
            Err(e) => Err(Status::internal(e.to_string())),
        }
    }

    async fn add_row(&self, request: Request<AddRowRequest>) -> Result<Response<AddRowResponse>, Status> {
        let req = request.into_inner();
        let schema = if req.schema.is_empty() { "sys" } else { &req.schema };
        let row = req.row.ok_or_else(|| Status::invalid_argument("Row is required"))?;
        let db_row = convert_row_from_rpc(row);
        
        self.check_permission(schema, Some(&req.table_name), PermissionAction::AddRow)?;
        
        match self.service.add_row(schema, &req.table_name, &db_row).await {
            Ok(row_id) => Ok(Response::new(AddRowResponse {
                success: true,
                row_id,
                message: String::new(),
            })),
            Err(e) => Err(Status::internal(e.to_string())),
        }
    }

    async fn get_row(&self, request: Request<GetRowRequest>) -> Result<Response<GetRowResponse>, Status> {
        let req = request.into_inner();
        let schema = if req.schema.is_empty() { "sys" } else { &req.schema };
        
        self.check_permission(schema, Some(&req.table_name), PermissionAction::GetRow)?;
        
        match self.service.get_row(schema, &req.table_name, req.row_id).await {
            Ok(Some(row)) => Ok(Response::new(GetRowResponse {
                success: true,
                row: Some(convert_row_to_rpc(&row)),
                message: String::new(),
            })),
            Ok(None) => Ok(Response::new(GetRowResponse {
                success: true,
                row: None,
                message: "Row not found".to_string(),
            })),
            Err(e) => Err(Status::internal(e.to_string())),
        }
    }

    async fn delete_row(&self, request: Request<DeleteRowRequest>) -> Result<Response<DeleteRowResponse>, Status> {
        let req = request.into_inner();
        let schema = if req.schema.is_empty() { "sys" } else { &req.schema };
        
        self.check_permission(schema, Some(&req.table_name), PermissionAction::DeleteRow)?;
        
        match self.service.delete_row(schema, &req.table_name, req.row_id).await {
            Ok(()) => Ok(Response::new(DeleteRowResponse {
                success: true,
                message: String::new(),
            })),
            Err(e) => Err(Status::internal(e.to_string())),
        }
    }

    async fn update_row(&self, request: Request<UpdateRowRequest>) -> Result<Response<UpdateRowResponse>, Status> {
        let req = request.into_inner();
        let schema = if req.schema.is_empty() { "sys" } else { &req.schema };
        let row = req.row.ok_or_else(|| Status::invalid_argument("Row is required"))?;
        let db_row = convert_row_from_rpc(row);
        
        self.check_permission(schema, Some(&req.table_name), PermissionAction::UpdateRow)?;
        
        match self.service.update_row(schema, &req.table_name, req.row_id, &db_row).await {
            Ok(()) => Ok(Response::new(UpdateRowResponse {
                success: true,
                message: String::new(),
            })),
            Err(e) => Err(Status::internal(e.to_string())),
        }
    }

    async fn get_all_meta(&self, request: Request<GetAllMetaRequest>) -> Result<Response<GetAllMetaResponse>, Status> {
        let req = request.into_inner();
        let schema = if req.schema.is_empty() { "sys" } else { &req.schema };
        
        self.check_permission(schema, None, PermissionAction::GetAllMeta)?;
        
        match self.service.get_all_meta(schema).await {
            Ok(meta_json) => Ok(Response::new(GetAllMetaResponse {
                success: true,
                meta_json,
                message: String::new(),
            })),
            Err(e) => Err(Status::internal(e.to_string())),
        }
    }

    async fn get_schema_info(&self, request: Request<GetSchemaInfoRequest>) -> Result<Response<GetSchemaInfoResponse>, Status> {
        let req = request.into_inner();
        let schema = if req.schema.is_empty() { "sys" } else { &req.schema };
        
        self.check_permission(schema, None, PermissionAction::GetSchemaInfo)?;
        
        match self.service.get_schema_info(schema).await {
            Ok(info_json) => Ok(Response::new(GetSchemaInfoResponse {
                success: true,
                info_json,
                message: String::new(),
            })),
            Err(e) => Err(Status::internal(e.to_string())),
        }
    }

    async fn list_schemas(&self, request: Request<ListSchemasRequest>) -> Result<Response<ListSchemasResponse>, Status> {
        let _req = request.into_inner();
        
        self.check_permission("sys", None, PermissionAction::ListTables)?;
        
        match self.service.list_schemas().await {
            Ok(schemas) => Ok(Response::new(ListSchemasResponse {
                success: true,
                schemas,
                message: String::new(),
            })),
            Err(e) => Err(Status::internal(e.to_string())),
        }
    }

    async fn get_version(&self, request: Request<GetVersionRequest>) -> Result<Response<GetVersionResponse>, Status> {
        let _req = request.into_inner();
        
        Ok(Response::new(GetVersionResponse {
            success: true,
            version: env!("CARGO_PKG_VERSION").to_string(),
            build_info: format!("Rust {}", rustc_version_runtime::version()),
            message: String::new(),
        }))
    }

    async fn get_table_meta(&self, request: Request<GetTableMetaRequest>) -> Result<Response<GetTableMetaResponse>, Status> {
        let req = request.into_inner();
        let schema = if req.schema.is_empty() { "sys" } else { &req.schema };
        
        self.check_permission(schema, Some(&req.table_name), PermissionAction::GetTableMeta)?;
        
        match self.service.get_table_meta(schema, &req.table_name).await {
            Ok(Some(meta)) => Ok(Response::new(GetTableMetaResponse {
                success: true,
                table_id: meta.table_id,
                table_name: meta.table_name,
                column_count: meta.column_count,
                message: String::new(),
            })),
            Ok(None) => Err(Status::not_found("Table not found")),
            Err(e) => Err(Status::internal(e.to_string())),
        }
    }

    async fn query(&self, request: Request<QueryRequest>) -> Result<Response<QueryResponse>, Status> {
        let req = request.into_inner();
        let schema = if req.schema.is_empty() { "sys" } else { &req.schema };
        
        self.check_permission(schema, None, PermissionAction::Query)?;
        
        let db_query = convert_query_from_rpc(&req);
        
        match self.service.query(schema, &db_query).await {
            Ok(result) => {
                let rows: Vec<_> = result.rows.iter().map(convert_query_row_to_rpc).collect();
                Ok(Response::new(QueryResponse {
                    success: true,
                    rows,
                    message: String::new(),
                }))
            },
            Err(e) => Err(Status::internal(e.to_string())),
        }
    }

    async fn sql_query(&self, request: Request<SqlQueryRequest>) -> Result<Response<SqlQueryResponse>, Status> {
        let req = request.into_inner();
        let schema = if req.schema.is_empty() { "sys" } else { &req.schema };
        let sql = req.sql.as_str();
        
        self.check_permission(schema, None, PermissionAction::Query)?;
        
        match self.service.sql_query(schema, sql).await {
            Ok(result) => {
                let mut columns: Vec<String> = Vec::new();
                let mut rows: Vec<SqlQueryResultRow> = Vec::new();
                
                if !result.rows.is_empty() {
                    if let Some(first_row) = result.rows.first() {
                        if first_row.row.is_some() {
                            let row = first_row.row.get_or_default();
                            for (idx, _) in row.data.iter().enumerate() {
                                columns.push(format!("col_{}", idx));
                            }
                        }
                    }
                    
                    for qr in &result.rows {
                        if qr.row.is_some() {
                            let row = qr.row.get_or_default();
                            let mut fields: Vec<SqlField> = Vec::new();
                            for data in &row.data {
                                use protobuf::CodedInputStream;
                                use laoflchdb_engines::Message;
                                let mut input = CodedInputStream::from_bytes(data);
                                if let Ok(field) = laoflchdb_engines::Field::parse_from(&mut input) {
                                    use laoflchdb_engines::field::field::Value;
                                    let sql_field = match field.value {
                                        Some(Value::StringValue(s)) => SqlField {
                                            value: Some(crate::pb::rpc::sql_field::Value::StringValue(s.value)),
                                        },
                                        Some(Value::IntegerValue(i)) => SqlField {
                                            value: Some(crate::pb::rpc::sql_field::Value::Int64Value(i.value)),
                                        },
                                        Some(Value::FloatValue(f)) => SqlField {
                                            value: Some(crate::pb::rpc::sql_field::Value::FloatValue(f.value)),
                                        },
                                        Some(Value::BytesValue(b)) => SqlField {
                                            value: Some(crate::pb::rpc::sql_field::Value::BytesValue(b.value)),
                                        },
                                        _ => SqlField {
                                            value: Some(crate::pb::rpc::sql_field::Value::StringValue(String::new())),
                                        },
                                    };
                                    fields.push(sql_field);
                                } else {
                                    if let Ok(s) = String::from_utf8(data.clone()) {
                                        if let Ok(num) = s.parse::<i64>() {
                                            fields.push(SqlField {
                                                value: Some(crate::pb::rpc::sql_field::Value::Int64Value(num)),
                                            });
                                        } else if let Ok(f) = s.parse::<f64>() {
                                            fields.push(SqlField {
                                                value: Some(crate::pb::rpc::sql_field::Value::FloatValue(f)),
                                            });
                                        } else {
                                            fields.push(SqlField {
                                                value: Some(crate::pb::rpc::sql_field::Value::StringValue(s)),
                                            });
                                        }
                                    } else {
                                        fields.push(SqlField {
                                            value: Some(crate::pb::rpc::sql_field::Value::BytesValue(data.clone())),
                                        });
                                    }
                                }
                            }
                            rows.push(SqlQueryResultRow { values: fields });
                        }
                    }
                }
                
                Ok(Response::new(SqlQueryResponse {
                    success: true,
                    rows,
                    columns,
                    message: String::new(),
                }))
            },
            Err(e) => Err(Status::internal(e.to_string())),
        }
    }
    
    async fn refresh_tables(&self, request: Request<RefreshTablesRequest>) -> Result<Response<RefreshTablesResponse>, Status> {
        let req = request.into_inner();
        let schema = if req.schema.is_empty() { "sys" } else { &req.schema };
        
        self.check_permission(schema, None, PermissionAction::Query)?;
        
        match self.service.refresh_tables(schema).await {
            Ok(tables) => Ok(Response::new(RefreshTablesResponse {
                success: true,
                tables,
                message: String::new(),
            })),
            Err(e) => Err(Status::internal(e.to_string())),
        }
    }
}
