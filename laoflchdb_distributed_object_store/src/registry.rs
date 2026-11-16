//! raw_store 节点注册表：静态种子 + Raft 注册发现 + 健康检查 + 一致性哈希环

use crate::config::DistributedObjectStoreConfig;
use crate::placement::ConsistentHashRing;
use crate::raft_meta::RaftMetaClient;
use laoflchdb_raw_store_service::client::RawStoreClientPool;
use laoflchdb_raw_store_service::registry::{now_secs, RegistryEntry};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

const REGISTRY_PREFIX: &[u8] = b"r\0";

pub struct RawStoreRegistry {
    seeds: Vec<String>,
    registry_group: RaftMetaClient,
    ttl_secs: u64,
    vnodes: usize,
    pool: Arc<RawStoreClientPool>,
    nodes: RwLock<Vec<String>>,
    ring: RwLock<ConsistentHashRing>,
}

impl RawStoreRegistry {
    pub async fn new(
        config: &DistributedObjectStoreConfig,
        pool: Arc<RawStoreClientPool>,
    ) -> Self {
        let registry_group = RaftMetaClient::new(
            config.raft_meta.endpoints.clone(),
            config.raft_meta.registry_group_id.clone(),
        );
        let seeds: Vec<String> = config
            .raw_store_seeds
            .iter()
            .map(|s| normalize_addr(s))
            .filter(|s| !s.is_empty())
            .collect();

        let registry = Self {
            seeds: seeds.clone(),
            registry_group,
            ttl_secs: config.registry_ttl_secs,
            vnodes: config.vnodes_per_node,
            pool,
            nodes: RwLock::new(seeds.clone()),
            ring: RwLock::new(ConsistentHashRing::new(&seeds, config.vnodes_per_node)),
        };

        registry.refresh().await;
        registry
    }

    /// 从静态种子与 Raft 注册表刷新节点列表，并重建哈希环
    pub async fn refresh(&self) {
        let mut addrs: Vec<String> = self.seeds.clone();

        match self.registry_group.scan().await {
            Ok(entries) => {
                let now = now_secs();
                for (key, value) in entries {
                    if !key.starts_with(REGISTRY_PREFIX) {
                        continue;
                    }
                    let entry: RegistryEntry = match serde_json::from_slice(&value) {
                        Ok(entry) => entry,
                        Err(e) => {
                            log::warn!("解析 raw_store 注册项失败: {e}");
                            continue;
                        }
                    };
                    if entry.addr.trim().is_empty() {
                        continue;
                    }
                    if self.ttl_secs > 0 && now.saturating_sub(entry.updated_at) > self.ttl_secs {
                        continue;
                    }
                    addrs.push(normalize_addr(&entry.addr));
                }
            }
            Err(e) => {
                log::debug!("读取 raw_store 注册表失败（继续使用静态种子）: {e}");
            }
        }

        addrs.sort();
        addrs.dedup();

        // 并发健康检查
        let pool = self.pool.clone();
        let checks = addrs.iter().map(|addr| {
            let addr = addr.clone();
            let pool = pool.clone();
            async move {
                let healthy = tokio::time::timeout(Duration::from_secs(2), pool.health(&addr))
                    .await
                    .map(|r| r.is_ok())
                    .unwrap_or(false);
                (addr, healthy)
            }
        });

        let mut healthy: Vec<String> = Vec::new();
        for (addr, ok) in futures_util::future::join_all(checks).await {
            if ok {
                healthy.push(addr);
            }
        }

        if healthy.is_empty() && !addrs.is_empty() {
            log::warn!("raw_store 健康检查全部失败，暂时保留候选节点");
            healthy = addrs;
        }

        if healthy != *self.nodes.read().await {
            log::info!("raw_store 节点列表更新: {:?}", healthy);
        }

        *self.nodes.write().await = healthy.clone();
        *self.ring.write().await = ConsistentHashRing::new(&healthy, self.vnodes);
    }

    pub async fn nodes(&self) -> Vec<String> {
        self.nodes.read().await.clone()
    }

    pub async fn replicas_for(&self, key: &str, count: usize) -> Vec<String> {
        self.ring.read().await.replicas(key, count)
    }

    pub fn spawn_refresh(self: Arc<Self>, interval_secs: u64) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let interval = Duration::from_secs(interval_secs.max(1));
            loop {
                tokio::time::sleep(interval).await;
                self.refresh().await;
            }
        })
    }
}

fn normalize_addr(addr: &str) -> String {
    addr.trim()
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/')
        .to_string()
}
