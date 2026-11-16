//! laoflchdb_distributed_object_store
//!
//! 分布式对象存储网关：每个网关节点提供完整 S3 协议接入，
//! 对象按固定大小分块后通过一致性哈希放置到全部 raw_store 节点，
//! 元数据统一写入现有 Raft 集群。

pub mod backend;
pub mod chunk_io;
pub mod config;
pub mod error;
pub mod placement;
pub mod raft_meta;
pub mod registry;
pub mod s3_protocol;

use backend::DistributedObjectStore;
use config::DistributedObjectStoreConfig;
use laoflchdb_raw_store_service::client::RawStoreClientPool;
use raft_meta::RaftMetaClient;
use registry::RawStoreRegistry;
use s3_protocol::S3Config;
use std::sync::Arc;

/// 启动分布式 S3 网关
pub async fn start_distributed_object_store(
    config: DistributedObjectStoreConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if config.listen_addr.trim().is_empty() {
        return Err("distributed_object_store.listen_addr 不能为空".into());
    }

    let pool = Arc::new(RawStoreClientPool::new());
    let registry = Arc::new(RawStoreRegistry::new(&config, pool.clone()).await);
    let _refresh = registry
        .clone()
        .spawn_refresh(config.registry_refresh_secs);

    let meta = RaftMetaClient::new(
        config.raft_meta.endpoints.clone(),
        config.raft_meta.group_id.clone(),
    );

    let backend = Arc::new(DistributedObjectStore::new(
        meta,
        registry,
        pool,
        Arc::new(config.clone()),
    ));

    let s3_config = S3Config {
        access_key: config.s3_access_key.clone(),
        secret_key: config.s3_secret_key.clone(),
        region: config.s3_region.clone(),
    };
    let router = s3_protocol::create_s3_router(backend, s3_config);

    let listener = tokio::net::TcpListener::bind(&config.listen_addr).await?;
    log::info!("分布式 S3 网关启动: {}", config.listen_addr);
    axum::serve(listener, router).await?;
    Ok(())
}
