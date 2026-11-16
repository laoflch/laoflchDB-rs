//! 分布式对象存储配置

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct DistributedObjectStoreConfig {
    #[serde(default)]
    pub enabled: bool,
    /// 分布式 S3 网关监听地址
    #[serde(default = "default_listen_addr")]
    pub listen_addr: String,
    #[serde(default = "default_access_key")]
    pub s3_access_key: String,
    #[serde(default = "default_secret_key")]
    pub s3_secret_key: String,
    #[serde(default = "default_region")]
    pub s3_region: String,
    /// 分块大小，默认 4MB
    #[serde(default = "default_chunk_size")]
    pub chunk_size_bytes: usize,
    /// 副本数，默认 3
    #[serde(default = "default_replication")]
    pub replication_factor: usize,
    /// 写成功所需最少副本数，默认 2
    #[serde(default = "default_quorum")]
    pub write_quorum: usize,
    /// raw_store 静态种子地址列表
    #[serde(default)]
    pub raw_store_seeds: Vec<String>,
    /// 注册信息过期时间（秒）
    #[serde(default = "default_registry_ttl")]
    pub registry_ttl_secs: u64,
    /// 注册表刷新间隔（秒）
    #[serde(default = "default_registry_refresh")]
    pub registry_refresh_secs: u64,
    /// 一致性哈希虚拟节点数
    #[serde(default = "default_vnodes")]
    pub vnodes_per_node: usize,
    /// Raft 元数据配置
    pub raft_meta: RaftMetaConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RaftMetaConfig {
    /// Raft 节点 gRPC 地址列表
    pub endpoints: Vec<String>,
    /// 对象 / 桶元数据所在组
    pub group_id: String,
    /// raw_store 节点注册表所在组
    #[serde(default = "default_registry_group")]
    pub registry_group_id: String,
}

fn default_listen_addr() -> String {
    "0.0.0.0:19001".to_string()
}
fn default_access_key() -> String {
    "admin".to_string()
}
fn default_secret_key() -> String {
    "laoflchdb".to_string()
}
fn default_region() -> String {
    "us-east-1".to_string()
}
fn default_chunk_size() -> usize {
    4 * 1024 * 1024
}
fn default_replication() -> usize {
    3
}
fn default_quorum() -> usize {
    2
}
fn default_registry_ttl() -> u64 {
    60
}
fn default_registry_refresh() -> u64 {
    10
}
fn default_vnodes() -> usize {
    128
}
fn default_registry_group() -> String {
    "raw_registry".to_string()
}

impl DistributedObjectStoreConfig {
    /// 实际使用的写 quorum（不超过副本数，至少为 1）
    pub fn effective_write_quorum(&self) -> usize {
        self.write_quorum
            .max(1)
            .min(self.replication_factor.max(1))
    }
}
