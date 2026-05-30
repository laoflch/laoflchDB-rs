use crate::service::DatabaseService;
use crate::pb::rpc::{
    laoflch_db_server::LaoflchDb,
    GetRequest, GetResponse,
    PutRequest, PutResponse,
    DeleteRequest, DeleteResponse,
    CreateTableRequest, CreateTableResponse,
    ListTablesRequest, ListTablesResponse,
    GetTableMetaRequest, GetTableMetaResponse,
};
use crate::db_engine::pb::ColumnType;
use std::sync::Arc;
use tonic::{Request, Response, Status};

pub mod rest;
pub use rest::RestService;

#[derive(Clone)]
pub struct GrpcService {
    service: Arc<dyn DatabaseService>,
}

impl GrpcService {
    pub fn new(service: Arc<dyn DatabaseService>) -> Self {
        Self { service }
    }
}

pub struct AccessService {
    service: Arc<dyn DatabaseService>,
}

impl AccessService {
    pub fn new(service: Arc<dyn DatabaseService>) -> Self {
        Self { service }
    }

    pub fn get_grpc_service(&self) -> GrpcService {
        GrpcService::new(Arc::clone(&self.service))
    }

    pub fn get_rest_service(&self) -> RestService {
        RestService::new(Arc::clone(&self.service))
    }

    pub fn get_service(&self) -> Arc<dyn DatabaseService> {
        Arc::clone(&self.service)
    }
}

#[tonic::async_trait]
impl LaoflchDb for GrpcService {
    async fn get(&self, request: Request<GetRequest>) -> Result<Response<GetResponse>, Status> {
        let req = request.into_inner();
        let schema = if req.schema.is_empty() { "sys" } else { &req.schema };
        
        match self.service.get(schema, &req.table, &req.key) {
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
        
        match self.service.put(schema, &req.table, &req.key, &req.value) {
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
        
        match self.service.delete(schema, &req.table, &req.key) {
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

        match self.service.create_table(schema, table_name, &columns_ref) {
            Ok(table_id) => Ok(Response::new(CreateTableResponse {
                success: true,
                table_id,
                message: String::new(),
            })),
            Err(e) => Err(Status::internal(e.to_string())),
        }
    }

    async fn list_tables(&self, request: Request<ListTablesRequest>) -> Result<Response<ListTablesResponse>, Status> {
        let req = request.into_inner();
        let schema = if req.schema.is_empty() { "sys" } else { &req.schema };
        
        match self.service.list_tables(schema) {
            Ok(tables) => Ok(Response::new(ListTablesResponse {
                success: true,
                tables,
                message: String::new(),
            })),
            Err(e) => Err(Status::internal(e.to_string())),
        }
    }

    async fn get_table_meta(&self, request: Request<GetTableMetaRequest>) -> Result<Response<GetTableMetaResponse>, Status> {
        let req = request.into_inner();
        let schema = if req.schema.is_empty() { "sys" } else { &req.schema };
        
        match self.service.get_table_meta(schema, &req.table_name) {
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
}
