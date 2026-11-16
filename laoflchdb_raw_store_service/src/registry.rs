//! raw_store 节点向现有 Raft KV 注册自身地址。
//!
//! 注册键：`r\0{node_id}`，值为 [`RegistryEntry`] 的 JSON。
//! S3 网关从 `raw_registry` 组扫描该前缀来发现节点。

use crate::RaftRegistryConfig;
use laoflchdb_raft_service::proto::raft_kv_service_client::RaftKvServiceClient;
use laoflchdb_raft_service::proto::PutRequest;
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryEntry {
    pub node_id: u64,
    pub addr: String,
    pub updated_at: u64,
}

pub fn registry_key(node_id: u64) -> Vec<u8> {
    format!("r\0{node_id}").into_bytes()
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// 启动周期性注册任务
pub fn spawn_registration(
    config: RaftRegistryConfig,
    node_id: u64,
    addr: String,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if addr.trim().is_empty() {
            log::warn!("raw_store 未配置 advertise_addr，跳过 Raft 注册");
            return;
        }
        if config.endpoints.is_empty() {
            log::warn!("raw_store 未配置 raft_registry.endpoints，跳过 Raft 注册");
            return;
        }

        let interval = config.register_interval_secs.max(1);
        loop {
            match put_registry(&config, node_id, &addr).await {
                Ok(()) => log::debug!("raw_store 已注册到 Raft: node_id={node_id}, addr={addr}"),
                Err(e) => log::warn!("raw_store 注册失败: {e}"),
            }
            tokio::time::sleep(Duration::from_secs(interval)).await;
        }
    })
}

async fn put_registry(
    config: &RaftRegistryConfig,
    node_id: u64,
    addr: &str,
) -> Result<(), String> {
    let entry = RegistryEntry {
        node_id,
        addr: addr.to_string(),
        updated_at: now_secs(),
    };
    let value = serde_json::to_vec(&entry).map_err(|e| format!("序列化注册信息失败: {e}"))?;
    let key = registry_key(node_id);

    let mut last_error = String::from("没有可用的 Raft endpoint");
    for endpoint in &config.endpoints {
        let url = if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
            endpoint.clone()
        } else {
            format!("http://{endpoint}")
        };

        let mut client = match RaftKvServiceClient::connect(url).await {
            Ok(client) => client,
            Err(e) => {
                last_error = format!("连接 Raft endpoint {endpoint} 失败: {e}");
                continue;
            }
        };

        let request = PutRequest {
            key: key.clone(),
            value: value.clone(),
            group_id: config.group_id.clone(),
        };

        match client.put(request).await {
            Ok(resp) => {
                let resp = resp.into_inner();
                if resp.success {
                    return Ok(());
                }
                last_error = format!("Raft 写入注册信息失败: {}", resp.message);
            }
            Err(e) => {
                last_error = format!("Raft Put RPC 失败: {e}");
            }
        }
    }

    Err(last_error)
}
