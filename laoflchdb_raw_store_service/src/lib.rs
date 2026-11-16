//! laoflchdb_raw_store_service
//!
//! raw_store 数据面节点的 gRPC 服务：把 chunk 直接落到本地文件系统。
//! 节点可通过现有 Raft KV 接口把自身地址注册到 `raw_registry` 组，供 S3 网关发现。

use laoflchdb_raw_storage_engine::{ChunkStorage, RawStorageConfig, RawStorageEngine};
use serde::Deserialize;
use std::sync::Arc;
use tonic::{Request, Response, Status};

pub mod client;
pub mod registry;

pub mod proto {
    tonic::include_proto!("laoflchdb.raw_store");
}

use proto::raw_store_service_server::{RawStoreService, RawStoreServiceServer};

/// gRPC 单条消息上限：默认 chunk 4MB + protobuf 开销，tonic 默认 4MB 会超限
const MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024;
use proto::{
    DeleteChunkRequest, DeleteChunkResponse, GetChunkRequest, GetChunkResponse, HealthRequest,
    HealthResponse, PutChunkRequest, PutChunkResponse, StatChunkRequest, StatChunkResponse,
};

/// raw_store 节点配置
#[derive(Debug, Clone, Deserialize)]
pub struct RawStoreConfig {
    #[serde(default)]
    pub enabled: bool,
    /// gRPC 监听地址
    #[serde(default = "default_address")]
    pub address: String,
    /// 节点编号（用于注册）
    #[serde(default)]
    pub node_id: u64,
    /// 对外可访问地址（注册到 Raft 的地址，形如 raw-store-1:29998）
    #[serde(default)]
    pub advertise_addr: String,
    /// chunk 文件根目录
    #[serde(default = "default_data_path")]
    pub data_path: String,
    #[serde(default = "default_true")]
    pub fsync: bool,
    #[serde(default = "default_shard_depth")]
    pub shard_depth: usize,
    /// 可选的 Raft 注册配置
    #[serde(default)]
    pub raft_registry: Option<RaftRegistryConfig>,
}

fn default_address() -> String {
    "0.0.0.0:29998".to_string()
}
fn default_data_path() -> String {
    "./laoflch_raw_store".to_string()
}
fn default_true() -> bool {
    true
}
fn default_shard_depth() -> usize {
    2
}
fn default_register_interval() -> u64 {
    10
}

/// raw_store 向 Raft 注册自身的配置
#[derive(Debug, Clone, Deserialize)]
pub struct RaftRegistryConfig {
    /// Raft 节点 gRPC 地址列表
    pub endpoints: Vec<String>,
    /// 注册使用的 Raft 组
    #[serde(default = "default_registry_group")]
    pub group_id: String,
    #[serde(default = "default_register_interval")]
    pub register_interval_secs: u64,
}

fn default_registry_group() -> String {
    "raw_registry".to_string()
}

/// raw_store gRPC 服务实现
pub struct RawStoreServiceImpl {
    engine: Arc<dyn ChunkStorage>,
    node_id: u64,
    address: String,
    advertise_addr: String,
}

impl RawStoreServiceImpl {
    pub fn from_config(
        config: &RawStoreConfig,
    ) -> Result<Arc<Self>, Box<dyn std::error::Error + Send + Sync>> {
        let engine = RawStorageEngine::new(&RawStorageConfig {
            root: config.data_path.clone(),
            fsync: config.fsync,
            shard_depth: config.shard_depth,
        })?;
        Ok(Arc::new(Self {
            engine: Arc::new(engine),
            node_id: config.node_id,
            address: config.address.clone(),
            advertise_addr: config.advertise_addr.clone(),
        }))
    }

    pub fn new(engine: Arc<dyn ChunkStorage>, node_id: u64, address: String, advertise_addr: String) -> Self {
        Self {
            engine,
            node_id,
            address,
            advertise_addr,
        }
    }

    pub fn address(&self) -> &str {
        &self.address
    }

    pub fn advertise_addr(&self) -> &str {
        &self.advertise_addr
    }

    pub fn node_id(&self) -> u64 {
        self.node_id
    }
}

#[tonic::async_trait]
impl RawStoreService for RawStoreServiceImpl {
    async fn put_chunk(
        &self,
        request: Request<PutChunkRequest>,
    ) -> Result<Response<PutChunkResponse>, Status> {
        let req = request.into_inner();
        self.engine
            .put_chunk(&req.chunk_id, &req.data)
            .await
            .map_err(|e| Status::internal(format!("写入 chunk 失败: {e}")))?;
        Ok(Response::new(PutChunkResponse {
            success: true,
            message: String::new(),
        }))
    }

    async fn get_chunk(
        &self,
        request: Request<GetChunkRequest>,
    ) -> Result<Response<GetChunkResponse>, Status> {
        let req = request.into_inner();
        match self.engine.get_chunk(&req.chunk_id).await {
            Ok(Some(data)) => Ok(Response::new(GetChunkResponse {
                found: true,
                data,
                message: String::new(),
            })),
            Ok(None) => Ok(Response::new(GetChunkResponse {
                found: false,
                data: Vec::new(),
                message: "chunk 不存在".to_string(),
            })),
            Err(e) => Err(Status::internal(format!("读取 chunk 失败: {e}"))),
        }
    }

    async fn delete_chunk(
        &self,
        request: Request<DeleteChunkRequest>,
    ) -> Result<Response<DeleteChunkResponse>, Status> {
        let req = request.into_inner();
        self.engine
            .delete_chunk(&req.chunk_id)
            .await
            .map_err(|e| Status::internal(format!("删除 chunk 失败: {e}")))?;
        Ok(Response::new(DeleteChunkResponse {
            success: true,
            message: String::new(),
        }))
    }

    async fn stat_chunk(
        &self,
        request: Request<StatChunkRequest>,
    ) -> Result<Response<StatChunkResponse>, Status> {
        let req = request.into_inner();
        match self.engine.stat_chunk(&req.chunk_id).await {
            Ok(Some(stat)) => Ok(Response::new(StatChunkResponse {
                found: true,
                size: stat.size,
                sha256: stat.sha256.to_vec(),
            })),
            Ok(None) => Ok(Response::new(StatChunkResponse {
                found: false,
                size: 0,
                sha256: Vec::new(),
            })),
            Err(e) => Err(Status::internal(format!("查询 chunk 失败: {e}"))),
        }
    }

    async fn health(
        &self,
        _request: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        self.engine
            .health()
            .await
            .map_err(|e| Status::internal(format!("raw_store 健康检查失败: {e}")))?;
        Ok(Response::new(HealthResponse {
            healthy: true,
            node_id: self.node_id,
            addr: self.advertise_addr.clone(),
        }))
    }
}

/// 启动 raw_store gRPC 服务
pub async fn start_raw_store_service(
    service: Arc<RawStoreServiceImpl>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr: std::net::SocketAddr = service.address().parse()?;
    log::info!(
        "raw_store gRPC 服务启动: {} (node_id={}, advertise={})",
        addr,
        service.node_id(),
        service.advertise_addr()
    );
    tonic::transport::Server::builder()
        .max_frame_size(16 * 1024 * 1024 - 1)
        .add_service(
            RawStoreServiceServer::from_arc(service)
                // tonic 默认单条消息上限 4MB，而默认 chunk 为 4MB 加上 protobuf 开销会超限，
                // 这里放宽到 16MB，与客户端保持一致
                .max_decoding_message_size(MAX_MESSAGE_SIZE)
                .max_encoding_message_size(MAX_MESSAGE_SIZE),
        )
        .serve(addr)
        .await?;
    Ok(())
}
