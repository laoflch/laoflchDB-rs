use crate::db::OltpDB;
use crate::pb::ColumnType as DbColType;
use crate::pb::laoflch_db_server::{LaoflchDb, LaoflchDbServer};
use crate::pb::{
    CreateTableRequest, CreateTableResponse, DeleteRequest, DeleteResponse, GetRequest,
    GetResponse, GetTableMetaRequest, GetTableMetaResponse, ListTablesRequest, ListTablesResponse,
    PutRequest, PutResponse,
};
use log::info;
use std::sync::Arc;
use tokio::sync::Mutex;
use tonic::{transport::Server, Request, Response, Status};

#[derive(Clone)]
pub struct LaoflchDbServiceImpl {
    pub db: Arc<Mutex<OltpDB>>,
}

impl LaoflchDbServiceImpl {
    pub fn new(db: OltpDB) -> Self {
        Self {
            db: Arc::new(Mutex::new(db)),
        }
    }

    pub fn into_server(self) -> LaoflchDbServer<Self> {
        LaoflchDbServer::new(self)
    }
}

#[tonic::async_trait]
impl LaoflchDb for LaoflchDbServiceImpl {
    async fn get(&self, request: Request<GetRequest>) -> Result<Response<GetResponse>, Status> {
        let r = request.into_inner();
        let db = self.db.lock().await;

        info!("gRPC[get]: table={}, key_len={}", r.table, r.key.len());

        match db.get_kv(&r.table, &r.key) {
            Ok(Some(v)) => Ok(Response::new(GetResponse { found: true, value: v })),
            Ok(None) => Ok(Response::new(GetResponse { found: false, value: vec![] })),
            Err(e) => Err(Status::internal(format!("db get error: {}", e))),
        }
    }

    async fn put(&self, request: Request<PutRequest>) -> Result<Response<PutResponse>, Status> {
        let r = request.into_inner();
        let db = self.db.lock().await;

        info!(
            "gRPC[put]: table={}, key_len={}, val_len={}",
            r.table,
            r.key.len(),
            r.value.len()
        );

        match db.put_kv(&r.table, &r.key, &r.value) {
            Ok(_) => Ok(Response::new(PutResponse {})),
            Err(e) => Err(Status::internal(format!("db put error: {}", e))),
        }
    }

    async fn delete(
        &self,
        request: Request<DeleteRequest>,
    ) -> Result<Response<DeleteResponse>, Status> {
        let r = request.into_inner();
        let db = self.db.lock().await;

        info!("gRPC[delete]: table={}, key_len={}", r.table, r.key.len());

        match db.delete_kv(&r.table, &r.key) {
            Ok(_) => Ok(Response::new(DeleteResponse {})),
            Err(e) => Err(Status::internal(format!("db delete error: {}", e))),
        }
    }

    async fn create_table(
        &self,
        request: Request<CreateTableRequest>,
    ) -> Result<Response<CreateTableResponse>, Status> {
        let r = request.into_inner();
        let mut db = self.db.lock().await;

        info!(
            "gRPC[create_table]: name={}, columns={}",
            r.table_name,
            r.columns.len()
        );

        let cols: Vec<(&str, DbColType)> = r
            .columns
            .iter()
            .map(|c| {
                (
                    c.name.as_str(),
                    match c.col_type {
                        1 => DbColType::Int64,
                        2 => DbColType::String,
                        3 => DbColType::Bytes,
                        _ => DbColType::Unknown,
                    },
                )
            })
            .collect();

        let table_id = db.create_table(&r.table_name, &cols);

        Ok(Response::new(CreateTableResponse { table_id }))
    }

    async fn list_tables(
        &self,
        _request: Request<ListTablesRequest>,
    ) -> Result<Response<ListTablesResponse>, Status> {
        let db = self.db.lock().await;
        let tables = db.list_tables();
        info!("gRPC[list_tables]: {} tables found", tables.len());
        Ok(Response::new(ListTablesResponse { tables }))
    }

    async fn get_table_meta(
        &self,
        _request: Request<GetTableMetaRequest>,
    ) -> Result<Response<GetTableMetaResponse>, Status> {
        Ok(Response::new(GetTableMetaResponse {
            exists: false,
            table_id: "".into(),
            table_name: "".into(),
            column_count: 0,
        }))
    }
}

pub async fn run_server(
    addr: &str,
    mut db: OltpDB,
) -> Result<(), Box<dyn std::error::Error>> {
    db.init_laoflch_db();

    let svc = LaoflchDbServiceImpl::new(db);

    info!("服务初始化完成，等待连接...");

    Server::builder()
        .add_service(svc.into_server())
        .serve(addr.parse()?)
        .await?;

    Ok(())
}
