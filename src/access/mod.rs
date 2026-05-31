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
    GetTableMetaRequest, GetTableMetaResponse,
    QueryRequest, QueryResponse,
    ColumnMeta as RpcColumnMeta,
    Row as RpcRow,
};
use crate::config::PermissionAction;
use laoflchdb_db_engine::pb::{ColumnMeta, Row};
use laoflchdb_db_engine::pb::ColumnType;
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

// 转换函数
fn convert_column_meta_to_rpc(meta: &ColumnMeta) -> RpcColumnMeta {
    RpcColumnMeta {
        table_id: meta.table_id,
        column_id: meta.column_id,
        column_name: meta.column_name.clone(),
        column_type: meta.column_type as i32,
    }
}

fn convert_row_from_rpc(rpc_row: RpcRow) -> Row {
    Row {
        row_type: rpc_row.row_type,
        version: rpc_row.version,
        data: rpc_row.data,
    }
}

fn convert_row_to_rpc(row: &Row) -> RpcRow {
    RpcRow {
        row_type: row.row_type,
        version: row.version,
        data: row.data.clone(),
    }
}

fn convert_query_from_rpc(req: &QueryRequest) -> laoflchdb_db_engine::pb::Query {
    use laoflchdb_db_engine::pb::{TableFilter, ColumnFilter, ColumnFilterCondition, FilterOperator, Field};
    
    let table_filters = req.table_filters.iter().map(|tf| {
        let column_filters = tf.column_filters.iter().map(|cf| {
            let conditions = cf.conditions.iter().map(|cond| {
                let op = match FilterOperator::from_i32(cond.op) {
                    Some(op) => op,
                    None => FilterOperator::Unspecified,
                };
                
                let value = cond.value.as_ref().map(|f| {
                    use laoflchdb_db_engine::pb::field::Value;
                    use laoflchdb_db_engine::pb::{String, Integer, Bytes, Float, List, Image};
                    
                    let val = match f.value {
                        Some(ref v) => match v {
                            crate::pb::rpc::field::Value::StringValue(s) => Value::StringValue(String {
                                value: s.value.clone(),
                            }),
                            crate::pb::rpc::field::Value::IntegerValue(i) => Value::IntegerValue(Integer {
                                value: i.value,
                            }),
                            crate::pb::rpc::field::Value::BytesValue(b) => Value::BytesValue(Bytes {
                                value: b.value.clone(),
                            }),
                            crate::pb::rpc::field::Value::FloatValue(fv) => Value::FloatValue(Float {
                                value: fv.value,
                            }),
                            crate::pb::rpc::field::Value::ListValue(l) => Value::ListValue(List {
                                items: l.items.clone(),
                            }),
                            crate::pb::rpc::field::Value::ImageValue(img) => Value::ImageValue(Image {
                                data: img.data.clone(),
                                format: img.format.clone(),
                            }),
                        },
                        None => Value::StringValue(String { value: std::string::String::new() }),
                    };
                    
                    Field { value: Some(val) }
                });
                
                let values = cond.values.iter().map(|f| {
                    use laoflchdb_db_engine::pb::field::Value;
                    use laoflchdb_db_engine::pb::{String, Integer, Bytes, Float, List, Image};
                    
                    let val = match f.value {
                        Some(ref v) => match v {
                            crate::pb::rpc::field::Value::StringValue(s) => Value::StringValue(String {
                                value: s.value.clone(),
                            }),
                            crate::pb::rpc::field::Value::IntegerValue(i) => Value::IntegerValue(Integer {
                                value: i.value,
                            }),
                            crate::pb::rpc::field::Value::BytesValue(b) => Value::BytesValue(Bytes {
                                value: b.value.clone(),
                            }),
                            crate::pb::rpc::field::Value::FloatValue(fv) => Value::FloatValue(Float {
                                value: fv.value,
                            }),
                            crate::pb::rpc::field::Value::ListValue(l) => Value::ListValue(List {
                                items: l.items.clone(),
                            }),
                            crate::pb::rpc::field::Value::ImageValue(img) => Value::ImageValue(Image {
                                data: img.data.clone(),
                                format: img.format.clone(),
                            }),
                        },
                        None => Value::StringValue(String { value: std::string::String::new() }),
                    };
                    
                    Field { value: Some(val) }
                }).collect();
                
                ColumnFilterCondition {
                    op: op as i32,
                    value,
                    values,
                }
            }).collect();
            
            ColumnFilter {
                column_name: cf.column_name.clone(),
                conditions,
            }
        }).collect();
        
        TableFilter {
            table_name: tf.table_name.clone(),
            column_filters,
        }
    }).collect();
    
    laoflchdb_db_engine::pb::Query {
        table_filters,
        limit: req.limit,
        offset: req.offset,
    }
}

fn convert_query_row_to_rpc(qr: &laoflchdb_db_engine::pb::QueryRow) -> crate::pb::rpc::QueryRow {
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
                let ct = ColumnType::try_from(col.column_type)
                    .unwrap_or(ColumnType::String);
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
        
        // 对于查询，我们检查权限（不指定具体表，因为查询可能涉及多个表）
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
}
