//! laoflchdb_raft_service - 基于 Raft 的分布式 KV 存储服务
//!
//! 使用 openraft 0.9.25 实现 Raft 共识，通过 RocksDB 持久化日志/元数据，gRPC 通信。

pub mod raft_types;
pub mod state_machine;
pub mod log_store;
pub mod network;
pub mod rest;
pub mod storage;

use crate::raft_types::{NodeId, Raft};
use crate::log_store::RaftLogStore;
use crate::network::{GrpcClientPool, GrpcNetworkFactory};
use crate::state_machine::RaftKvStateMachine;
use crate::storage::{business_table_name, RaftStorageConfig};
use laoflchdb_engines::KvStorage;
use openraft::Config;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tonic::{transport::Server, Request, Response, Status};

// ── 配置 ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RaftServiceConfig {
    /// 是否启用 Raft 服务
    #[serde(default)]
    pub enabled: bool,
    /// gRPC 服务地址（所有 Raft 组共用同一端口）
    #[serde(default = "default_address")]
    pub address: String,
    /// 数据存储路径（每个组使用 data_path/group-{group_id} 子目录）
    #[serde(default = "default_data_path")]
    pub data_path: String,
    /// Raft 选举超时（毫秒）
    #[serde(default = "default_election_timeout")]
    pub election_timeout: u64,
    /// Raft 心跳间隔（毫秒）
    #[serde(default = "default_heartbeat_interval")]
    pub heartbeat_interval: u64,
    /// 业务数据落库引擎配置（状态机真身，缺省 rocksdb，路径 data_path/business.db）。
    /// 组级 storage 配置可覆盖服务级。
    #[serde(default)]
    pub storage: Option<RaftStorageConfig>,
    /// 双写副本配置（可选）：开启后把业务数据同步写入另一个 KvStorage 引擎，
    /// 作为业务数据副本（与 Raft 复制解耦，供业务侧独立读取）。
    #[serde(default)]
    pub dual_write: Option<DualWriteConfig>,
    /// 多 Raft 组配置（每个组独立选举、独立日志与状态机，共用同一 gRPC 端口）
    #[serde(default)]
    pub groups: Vec<RaftGroupConfig>,
}

/// 双写副本写入模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DualWriteMode {
    /// 同步写入：副本写失败会向 Raft apply 返回错误（副本与主强一致，延迟受副本影响）
    Sync,
    /// 异步写入：spawn 后台任务写副本，失败仅记日志、不阻塞主路径（主流程延迟最小，副本最终一致）
    Async,
}

impl Default for DualWriteMode {
    fn default() -> Self {
        Self::Sync
    }
}

/// 双写副本配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DualWriteConfig {
    /// 是否启用双写
    #[serde(default)]
    pub enabled: bool,
    /// 副本写入模式：sync（默认）/ async
    #[serde(default)]
    pub mode: DualWriteMode,
    /// 副本存储引擎配置（缺省：与主引擎同类型，路径 data_path/business_backup.db）
    #[serde(default)]
    pub storage: Option<RaftStorageConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RaftGroupConfig {
    /// 组 ID（字符串，同一节点上必须唯一；仅允许字母/数字/下划线/连字符，长度 1..=64）
    pub group_id: String,
    /// 本节点在该组中的节点 ID（组内唯一）
    pub node_id: u64,
    /// 本节点在该组中的对外地址（如 "raft-node-1:29999"）。
    /// 缺省时使用 raft_service.address（监听地址，通常为 0.0.0.0，不适合作为对端连接地址，
    /// 会导致 follower 无法连接 leader）。
    #[serde(default)]
    pub self_addr: Option<String>,
    /// 该组全部节点的地址（含自身；bootstrap 节点初始化时写入完整成员）
    #[serde(default)]
    pub peers: Vec<PeerConfig>,
    /// 该组是否在本节点自举集群（每组应恰有一个节点为 true，其初始 leader 大概率落在该节点）
    #[serde(default)]
    pub bootstrap: bool,
    /// 该组的存储引擎配置（可选，覆盖服务级 raft_service.storage）
    #[serde(default)]
    pub storage: Option<RaftStorageConfig>,
    /// 组名称（可选，用于显示）
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerConfig {
    pub id: u64,
    pub addr: String,
}

fn default_address() -> String {
    "127.0.0.1:29999".to_string()
}

fn default_data_path() -> String {
    "./laoflch_raft_data".to_string()
}

fn default_election_timeout() -> u64 {
    1000
}

fn default_heartbeat_interval() -> u64 {
    100
}

// ── gRPC 请求/响应类型（与 proto 对应）──

pub mod proto {
    tonic::include_proto!("laoflchdb.raft");
}

// ── 服务实现 ──

/// 单个 Raft 组的运行时（raft 实例 + 共享状态机）
pub struct GroupRuntime {
    pub raft: Raft,
    pub state_machine: RaftKvStateMachine,
}

/// 校验 group_id：允许字母/数字/下划线/连字符，长度 1..=64。
/// group_id 会用作存储目录名（group-{id}）与 RocksDB 表名（group_{id}），因此必须限制字符集。
fn validate_group_id(group_id: &str) -> Result<(), String> {
    if group_id.is_empty() || group_id.len() > 64 {
        return Err(format!("group_id 长度必须在 1..=64: {:?}", group_id));
    }
    if !group_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(format!(
            "group_id 仅允许字母/数字/下划线/连字符: {:?}",
            group_id
        ));
    }
    Ok(())
}

pub struct RaftKvServiceImpl {
    config: RaftServiceConfig,
    /// group_id -> 组运行时。Raft 是 Arc<RaftInner>，本身可 Clone；
    /// 这里用 std RwLock 提供 interior mutability，以便 gRPC service 满足 Sync。
    groups: std::sync::RwLock<std::collections::HashMap<String, GroupRuntime>>,
    grpc_pool: GrpcClientPool,
}

impl RaftKvServiceImpl {
    pub async fn new(config: &RaftServiceConfig) -> Result<Self, String> {
        Ok(Self {
            config: config.clone(),
            groups: std::sync::RwLock::new(std::collections::HashMap::new()),
            grpc_pool: GrpcClientPool::new(),
        })
    }

    /// 初始化各组日志存储并启动 Raft 实例（含集群自举）
    pub async fn start(&self) -> Result<(), String> {
        if self.config.groups.is_empty() {
            return Err("raft_service.groups 未配置任何 Raft 组".to_string());
        }

        let group_configs = self.config.groups.clone();
        for group_cfg in &group_configs {
            validate_group_id(&group_cfg.group_id)?;
        }
        let business_path = PathBuf::from(&self.config.data_path).join("business.db");
        let backup_path = PathBuf::from(&self.config.data_path).join("business_backup.db");
        let dual_write_cfg = self.config.dual_write.clone().filter(|d| d.enabled);
        // 副本写入模式（sync / async），缺省同步
        let dual_mode = dual_write_cfg
            .as_ref()
            .map(|d| d.mode)
            .unwrap_or(DualWriteMode::Sync);

        // 打开存储引擎（按 引擎类型|db_path 去重，相同配置的组共享同一实例）。
        // 主引擎：状态机真身（table 概念对应 Raft 组，每组数据存放于 table = "group_{g}"）。
        // 副本引擎：双写时启用，作为业务数据副本。
        let mut engines: std::collections::HashMap<String, Arc<dyn KvStorage>> =
            std::collections::HashMap::new();
        let mut group_engines: std::collections::HashMap<String, Arc<dyn KvStorage>> =
            std::collections::HashMap::new();
        let mut group_dual: std::collections::HashMap<String, Arc<dyn KvStorage>> =
            std::collections::HashMap::new();

        for group_cfg in &group_configs {
            // 组级 storage 优先，否则服务级，否则默认（rocksdb）
            let storage_cfg = group_cfg
                .storage
                .clone()
                .or_else(|| self.config.storage.clone())
                .unwrap_or_default();
            let db_path = storage_cfg
                .db_path
                .clone()
                .unwrap_or_else(|| business_path.to_string_lossy().into_owned());
            let key = format!("{}|{}", storage_cfg.storage_type, db_path);
            let engine = if let Some(e) = engines.get(&key) {
                e.clone()
            } else {
                let engine = crate::storage::open_storage(&storage_cfg, &business_path)?;
                engines.insert(key, engine.clone());
                engine
            };
            // 确保本组的业务表存在（幂等）
            let table = business_table_name(&group_cfg.group_id);
            engine
                .ensure_table(&table)
                .await
                .map_err(|e| format!("确保业务表 {} 存在失败: {}", table, e))?;
            group_engines.insert(group_cfg.group_id.clone(), engine);

            // 双写副本引擎
            if let Some(dual) = &dual_write_cfg {
                let dual_cfg = dual.storage.clone().unwrap_or_else(|| RaftStorageConfig {
                    storage_type: storage_cfg.storage_type.clone(),
                    db_path: None,
                    schema_name: storage_cfg.schema_name.clone(),
                });
                let dual_db_path = dual_cfg
                    .db_path
                    .clone()
                    .unwrap_or_else(|| backup_path.to_string_lossy().into_owned());
                let dkey = format!("{}|{}", dual_cfg.storage_type, dual_db_path);
                let dual_engine = if let Some(e) = engines.get(&dkey) {
                    e.clone()
                } else {
                    let engine = crate::storage::open_storage(&dual_cfg, &backup_path)?;
                    engines.insert(dkey, engine.clone());
                    engine
                };
                dual_engine
                    .ensure_table(&table)
                    .await
                    .map_err(|e| format!("确保副本业务表 {} 存在失败: {}", table, e))?;
                group_dual.insert(group_cfg.group_id.clone(), dual_engine);
            }
        }

        for group_cfg in group_configs {
            let kvdb = group_engines
                .get(&group_cfg.group_id)
                .cloned()
                .expect("主存储引擎必须存在");
            let dual_write = group_dual.get(&group_cfg.group_id).cloned();
            self.start_group(&group_cfg, kvdb, dual_write, dual_mode).await?;
        }
        Ok(())
    }

    /// 启动单个 Raft 组
    async fn start_group(
        &self,
        group_cfg: &RaftGroupConfig,
        kvdb: Arc<dyn KvStorage>,
        dual_write: Option<Arc<dyn KvStorage>>,
        dual_mode: DualWriteMode,
    ) -> Result<(), String> {
        let group_id = group_cfg.group_id.clone();
        if self.groups.read().unwrap().contains_key(&group_id) {
            return Err(format!("group_id {} 重复配置", group_id));
        }

        // 每个组使用独立的 RocksDB 子目录，避免多个 Raft 实例争用同一数据库
        let data_path = PathBuf::from(&self.config.data_path).join(format!("group-{}", group_id));
        let log_store = RaftLogStore::open(&data_path)?;

        // 创建 Raft 配置（election_timeout_max 必须大于 min，且 min > heartbeat_interval）
        let raft_config = Config {
            cluster_name: format!("laoflchdb-g{}", group_id),
            election_timeout_min: self.config.election_timeout,
            election_timeout_max: self.config.election_timeout * 2,
            heartbeat_interval: self.config.heartbeat_interval,
            ..Default::default()
        };
        let raft_config = raft_config
            .validate()
            .map_err(|e| format!("Invalid raft config: {}", e))?;

        // 每个组使用独立的网络工厂（携带 group_id，内部 RPC 按组路由）
        let network_factory = GrpcNetworkFactory::new(self.grpc_pool.clone(), group_id.clone());
        let state_machine = RaftKvStateMachine::new(group_id.clone(), kvdb, dual_write, dual_mode);

        // 创建 Raft 实例
        let raft = Raft::new(
            group_cfg.node_id,
            Arc::new(raft_config),
            network_factory,
            log_store,
            state_machine.clone(),
        )
        .await
        .map_err(|e| format!("Failed to create Raft instance for group {}: {}", group_id, e))?;

        // 仅该组的 bootstrap 节点自举集群（只初始化自身为唯一投票者，立即当选 leader）；
        // 其余 peers 由后台任务 auto_join_peers 通过 add_learner + change_membership 加入。
        // 非 bootstrap 节点不 initialize，保持空日志状态，通过 leader 的日志复制学习成员关系。
        if group_cfg.bootstrap {
            let initialized = raft
                .is_initialized()
                .await
                .map_err(|e| format!("Failed to check initialization: {}", e))?;
            if !initialized {
                // 自身地址优先用 self_addr（容器内可达地址），缺省回退到监听地址
                let self_addr = group_cfg
                    .self_addr
                    .clone()
                    .unwrap_or_else(|| self.config.address.clone());
                let mut members = std::collections::BTreeMap::new();
                members.insert(
                    group_cfg.node_id,
                    openraft::BasicNode { addr: self_addr },
                );
                if let Err(e) = raft.initialize(members).await {
                    // NotAllowed 表示集群已初始化，可安全忽略
                    log::warn!("组 {} 初始化集群失败（可能已初始化）: {}", group_id, e);
                }
            }
            // 后台自动将配置中的 peers 加入集群并提升为投票成员
            let raft_h = raft.clone();
            let self_id = group_cfg.node_id;
            let peers = group_cfg.peers.clone();
            tokio::spawn(async move {
                auto_join_peers(raft_h, self_id, peers).await;
            });
        } else {
            log::info!("组 {} bootstrap=false，等待集群 leader 加入", group_id);
        }

        self.groups.write().unwrap().insert(
            group_id.clone(),
            GroupRuntime { raft, state_machine },
        );

        log::info!("Raft 组 {} 启动完成 (node_id={})", group_id, group_cfg.node_id);
        Ok(())
    }

    /// 获取指定组的 Raft 引用
    pub fn get_raft(&self, group_id: &str) -> Option<Raft> {
        self.groups.read().unwrap().get(group_id).map(|g| g.raft.clone())
    }

    /// 获取指定组的共享状态机
    pub fn get_state_machine(&self, group_id: &str) -> Option<RaftKvStateMachine> {
        self.groups.read().unwrap().get(group_id).map(|g| g.state_machine.clone())
    }

    /// 本节点参与的所有组 ID（已启动）
    pub fn group_ids(&self) -> Vec<String> {
        self.groups.read().unwrap().keys().cloned().collect()
    }

    // ── KV 操作（gRPC 与 REST 复用）──

    pub(crate) async fn kv_put(&self, group_id: &str, key: Vec<u8>, value: Vec<u8>) -> Result<bool, String> {
        // 在锁内 clone 出 Raft 句柄，避免 std::sync::RwLockReadGuard 跨 await 持有（非 Send）
        let raft = self
            .get_raft(group_id)
            .ok_or(format!("group {} 不存在或未启动", group_id))?;

        let request = crate::raft_types::Request::Put {
            key: key.clone(),
            value: value.clone(),
        };

        match raft.client_write(request).await {
            Ok(response) => Ok(response.data.success),
            Err(e) => {
                // 本节点不是 leader 时，openraft 会返回 ForwardToLeader（含 leader 地址）：
                // 自动转发给 leader
                if let Some(ftl) = e.forward_to_leader() {
                    if let Some(leader) = &ftl.leader_node {
                        log::info!("[group {}] 转发 put 到 leader {}", group_id, leader.addr);
                        return self.forward_put(group_id, &leader.addr, key, value).await;
                    }
                }
                Err(format!("Failed to write: {}", e))
            }
        }
    }

    pub(crate) async fn kv_get(&self, group_id: &str, key: &[u8]) -> Result<Option<Vec<u8>>, String> {
        // 直查状态机（即主存储引擎）
        let state_machine = self
            .get_state_machine(group_id)
            .ok_or(format!("group {} 不存在或未启动", group_id))?;
        state_machine.get(key).await
    }

    pub(crate) async fn kv_delete(&self, group_id: &str, key: Vec<u8>) -> Result<bool, String> {
        let raft = self
            .get_raft(group_id)
            .ok_or(format!("group {} 不存在或未启动", group_id))?;

        let request = crate::raft_types::Request::Delete {
            key: key.clone(),
        };

        match raft.client_write(request).await {
            Ok(response) => Ok(response.data.success),
            Err(e) => {
                // 本节点不是 leader 时自动转发给 leader
                if let Some(ftl) = e.forward_to_leader() {
                    if let Some(leader) = &ftl.leader_node {
                        log::info!("[group {}] 转发 delete 到 leader {}", group_id, leader.addr);
                        return self.forward_delete(group_id, &leader.addr, key).await;
                    }
                }
                Err(format!("Failed to delete: {}", e))
            }
        }
    }

    pub(crate) async fn kv_scan(&self, group_id: &str) -> Result<Vec<(Vec<u8>, Vec<u8>)>, String> {
        let state_machine = self
            .get_state_machine(group_id)
            .ok_or(format!("group {} 不存在或未启动", group_id))?;
        state_machine.scan().await
    }

    // ── 写请求转发（follower 收到写入时自动转发给 leader）──

    async fn forward_put(
        &self,
        group_id: &str,
        leader_addr: &str,
        key: Vec<u8>,
        value: Vec<u8>,
    ) -> Result<bool, String> {
        let channel = tonic::transport::Endpoint::from_shared(format!("http://{}", leader_addr))
            .map_err(|e| format!("leader 地址非法 {}: {}", leader_addr, e))?
            .connect()
            .await
            .map_err(|e| format!("连接 leader {} 失败: {}", leader_addr, e))?;
        let mut client = crate::proto::raft_kv_service_client::RaftKvServiceClient::new(channel);
        let resp = client
            .put(crate::proto::PutRequest {
                group_id: group_id.to_string(),
                key,
                value,
            })
            .await
            .map_err(|e| format!("转发 put 到 leader {} 失败: {}", leader_addr, e))?;
        Ok(resp.into_inner().success)
    }

    async fn forward_delete(
        &self,
        group_id: &str,
        leader_addr: &str,
        key: Vec<u8>,
    ) -> Result<bool, String> {
        let channel = tonic::transport::Endpoint::from_shared(format!("http://{}", leader_addr))
            .map_err(|e| format!("leader 地址非法 {}: {}", leader_addr, e))?
            .connect()
            .await
            .map_err(|e| format!("连接 leader {} 失败: {}", leader_addr, e))?;
        let mut client = crate::proto::raft_kv_service_client::RaftKvServiceClient::new(channel);
        let resp = client
            .delete(crate::proto::DeleteRequest {
                group_id: group_id.to_string(),
                key,
            })
            .await
            .map_err(|e| format!("转发 delete 到 leader {} 失败: {}", leader_addr, e))?;
        Ok(resp.into_inner().success)
    }

    // ── 集群管理 ──

    async fn cluster_add_peer(&self, group_id: &str, id: u64, addr: String) -> Result<bool, String> {
        let raft = self
            .get_raft(group_id)
            .ok_or(format!("group {} 不存在或未启动", group_id))?;

        let node = openraft::BasicNode { addr };
        raft.add_learner(id, node, true)
            .await
            .map_err(|e| format!("Failed to add learner: {}", e))?;

        // 将新节点升级为投票成员
        let voter_ids: std::collections::BTreeSet<u64> = {
            // 绑定 metrics receiver 到变量，避免临时值被提前 drop
            let rx = raft.metrics();
            let metrics = rx.borrow();
            let mut ids: std::collections::BTreeSet<u64> = metrics.membership_config.voter_ids().collect();
            ids.insert(id);
            ids
        };
        raft.change_membership(voter_ids, false)
            .await
            .map_err(|e| format!("Failed to change membership: {}", e))?;

        Ok(true)
    }

    async fn cluster_remove_peer(&self, group_id: &str, id: u64) -> Result<bool, String> {
        let raft = self
            .get_raft(group_id)
            .ok_or(format!("group {} 不存在或未启动", group_id))?;

        let members = std::collections::BTreeSet::from([id]);
        raft.change_membership(openraft::ChangeMembers::RemoveVoters(members), false)
            .await
            .map_err(|e| format!("Failed to remove peer: {}", e))?;

        Ok(true)
    }

    async fn cluster_list_peers(&self, group_id: &str) -> Result<Vec<(u64, String)>, String> {
        let raft = self
            .get_raft(group_id)
            .ok_or(format!("group {} 不存在或未启动", group_id))?;

        let rx = raft.metrics();
        let metrics = rx.borrow();
        let mut peers = Vec::new();
        for (id, node) in metrics.membership_config.nodes() {
            peers.push((*id, node.addr.clone()));
        }
        Ok(peers)
    }

    pub(crate) async fn node_status(&self, group_id: &str) -> Result<String, String> {
        let raft = self
            .get_raft(group_id)
            .ok_or(format!("group {} 不存在或未启动", group_id))?;

        let rx = raft.metrics();
        Ok(format!("{:?}", *rx.borrow()))
    }
}

/// bootstrap 节点后台任务：当选 leader 后，将配置中的 peers 逐个加入集群并提升为投票成员。
/// 设计说明：bootstrap 节点 initialize 时只含自身（单 voter，立即当选），
/// 若 initialize 时把全部成员写入 voters，未初始化的 Learner 节点不参与投票，
/// 将导致无法凑够多数票而选举失败（term 无限飙升）。
async fn auto_join_peers(raft: Raft, self_id: NodeId, peers: Vec<PeerConfig>) {
    const RETRY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
    let start = std::time::Instant::now();

    loop {
        if start.elapsed() > RETRY_TIMEOUT {
            log::error!("自动加入 peers 超时（120s），请手动通过 AddPeer 加入剩余节点");
            return;
        }

        // 等待本节点成为 leader
        let is_leader = {
            let rx = raft.metrics();
            let m = rx.borrow();
            m.state == openraft::ServerState::Leader
        };
        if !is_leader {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            continue;
        }

        let current_voters: std::collections::BTreeSet<u64> = {
            let rx = raft.metrics();
            let m = rx.borrow();
            m.membership_config.voter_ids().collect()
        };

        // 找出尚未加入的 peers
        let missing: Vec<PeerConfig> = peers
            .iter()
            .filter(|p| p.id != self_id && !current_voters.contains(&p.id))
            .cloned()
            .collect();
        if missing.is_empty() {
            log::info!("组成员已全部加入并提升为投票成员: {:?}", current_voters);
            return;
        }

        // 逐个 add_learner（阻塞等待日志提交）；任一失败则稍后整轮重试
        let mut all_ok = true;
        for p in &missing {
            if let Err(e) = raft
                .add_learner(p.id, openraft::BasicNode { addr: p.addr.clone() }, true)
                .await
            {
                log::warn!("add_learner({}, {}) 失败: {}，稍后重试", p.id, p.addr, e);
                all_ok = false;
                break;
            }
        }
        if !all_ok {
            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
            continue;
        }

        // 一次性将全部成员（含新 learner）提升为投票成员
        let mut all_voters = current_voters;
        for p in &missing {
            all_voters.insert(p.id);
        }
        match raft.change_membership(all_voters.clone(), false).await {
            Ok(_) => {
                log::info!("组成员已提升为投票成员: {:?}", all_voters);
            }
            Err(e) => {
                log::warn!("change_membership 失败: {}，稍后重试", e);
                tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
            }
        }
    }
}

// ── gRPC Service 实现 ──

#[tonic::async_trait]
impl proto::raft_kv_service_server::RaftKvService for RaftKvServiceImpl {
    async fn put(
        &self,
        request: Request<proto::PutRequest>,
    ) -> Result<Response<proto::PutResponse>, Status> {
        let req = request.into_inner();
        match self.kv_put(&req.group_id, req.key, req.value).await {
            Ok(success) => Ok(Response::new(proto::PutResponse {
                success,
                message: "OK".to_string(),
            })),
            Err(e) => Err(Status::internal(e)),
        }
    }

    async fn get(
        &self,
        request: Request<proto::GetRequest>,
    ) -> Result<Response<proto::GetResponse>, Status> {
        let req = request.into_inner();
        match self.kv_get(&req.group_id, &req.key).await {
            Ok(Some(value)) => Ok(Response::new(proto::GetResponse {
                found: true,
                value,
            })),
            Ok(None) => Ok(Response::new(proto::GetResponse {
                found: false,
                value: Vec::new(),
            })),
            Err(e) => Err(Status::internal(e)),
        }
    }

    async fn delete(
        &self,
        request: Request<proto::DeleteRequest>,
    ) -> Result<Response<proto::DeleteResponse>, Status> {
        let req = request.into_inner();
        match self.kv_delete(&req.group_id, req.key).await {
            Ok(success) => Ok(Response::new(proto::DeleteResponse { success })),
            Err(e) => Err(Status::internal(e)),
        }
    }

    async fn scan(
        &self,
        request: Request<proto::ScanRequest>,
    ) -> Result<Response<proto::ScanResponse>, Status> {
        let req = request.into_inner();
        match self.kv_scan(&req.group_id).await {
            Ok(entries) => {
                let kv_entries = entries
                    .into_iter()
                    .map(|(key, value)| proto::KvEntry { key, value })
                    .collect();
                Ok(Response::new(proto::ScanResponse { entries: kv_entries }))
            }
            Err(e) => Err(Status::internal(e)),
        }
    }

    async fn add_peer(
        &self,
        request: Request<proto::AddPeerRequest>,
    ) -> Result<Response<proto::AddPeerResponse>, Status> {
        let req = request.into_inner();
        match self.cluster_add_peer(&req.group_id, req.id, req.addr).await {
            Ok(success) => Ok(Response::new(proto::AddPeerResponse {
                success,
                message: "OK".to_string(),
            })),
            Err(e) => Err(Status::internal(e)),
        }
    }

    async fn remove_peer(
        &self,
        request: Request<proto::RemovePeerRequest>,
    ) -> Result<Response<proto::RemovePeerResponse>, Status> {
        let req = request.into_inner();
        match self.cluster_remove_peer(&req.group_id, req.id).await {
            Ok(success) => Ok(Response::new(proto::RemovePeerResponse {
                success,
                message: "OK".to_string(),
            })),
            Err(e) => Err(Status::internal(e)),
        }
    }

    async fn list_peers(
        &self,
        request: Request<proto::ListPeersRequest>,
    ) -> Result<Response<proto::ListPeersResponse>, Status> {
        let req = request.into_inner();
        match self.cluster_list_peers(&req.group_id).await {
            Ok(peers) => {
                let peer_infos = peers
                    .into_iter()
                    .map(|(id, addr)| proto::PeerInfo { id, addr })
                    .collect();
                Ok(Response::new(proto::ListPeersResponse { peers: peer_infos }))
            }
            Err(e) => Err(Status::internal(e)),
        }
    }

    async fn get_status(
        &self,
        request: Request<proto::GetStatusRequest>,
    ) -> Result<Response<proto::GetStatusResponse>, Status> {
        let req = request.into_inner();
        match self.node_status(&req.group_id).await {
            Ok(status) => Ok(Response::new(proto::GetStatusResponse { status })),
            Err(e) => Err(Status::internal(e)),
        }
    }

    // ── Raft 内部 RPC 处理（节点间通信）──

    async fn request_vote(
        &self,
        request: Request<proto::RequestVoteRequest>,
    ) -> Result<Response<proto::RequestVoteResponse>, Status> {
        let inner = request.into_inner();
        let raft = self
            .get_raft(&inner.group_id)
            .ok_or_else(|| Status::failed_precondition(format!("group {} 不存在或未启动", inner.group_id)))?;

        let rpc: openraft::raft::VoteRequest<NodeId> = bincode::deserialize(&inner.payload)
            .map_err(|e| Status::invalid_argument(format!("Failed to deserialize VoteRequest: {}", e)))?;

        let resp = raft
            .vote(rpc)
            .await
            .map_err(|e| Status::internal(format!("RequestVote RPC error: {}", e)))?;

        let payload = bincode::serialize(&resp)
            .map_err(|e| Status::internal(format!("Failed to serialize VoteResponse: {}", e)))?;
        Ok(Response::new(proto::RequestVoteResponse { payload }))
    }

    async fn append_entries(
        &self,
        request: Request<proto::AppendEntriesRequest>,
    ) -> Result<Response<proto::AppendEntriesResponse>, Status> {
        let inner = request.into_inner();
        let raft = self
            .get_raft(&inner.group_id)
            .ok_or_else(|| Status::failed_precondition(format!("group {} 不存在或未启动", inner.group_id)))?;

        log::info!(
            "[raft-svc] g{} append_entries req_payload_len={}",
            inner.group_id,
            inner.payload.len()
        );
        let rpc: openraft::raft::AppendEntriesRequest<crate::raft_types::TypeConfig> =
            bincode::deserialize(&inner.payload)
                .map_err(|e| {
                    log::error!(
                        "[raft-svc] g{} append_entries req deserialize FAILED len={} hex={:02x?}",
                        inner.group_id,
                        inner.payload.len(),
                        &inner.payload[..inner.payload.len().min(96)]
                    );
                    Status::invalid_argument(format!("Failed to deserialize AppendEntriesRequest: {}", e))
                })?;

        // 取证日志：follower 实际收到的 prev_log_id / entries 范围 / leader_commit
        log::info!(
            "[raft-svc] g{} append_entries prev={:?} entries={} first={:?} last={:?} commit={:?}",
            inner.group_id,
            rpc.prev_log_id,
            rpc.entries.len(),
            rpc.entries.first().map(|e| e.log_id.index),
            rpc.entries.last().map(|e| e.log_id.index),
            rpc.leader_commit
        );

        let resp = raft
            .append_entries(rpc)
            .await
            .map_err(|e| Status::internal(format!("AppendEntries RPC error: {}", e)))?;

        let payload = bincode::serialize(&resp)
            .map_err(|e| Status::internal(format!("Failed to serialize AppendEntriesResponse: {}", e)))?;
        log::info!(
            "[raft-svc] g{} append_entries resp_payload_len={}",
            inner.group_id,
            payload.len()
        );
        Ok(Response::new(proto::AppendEntriesResponse { payload }))
    }

    async fn install_snapshot(
        &self,
        request: Request<proto::InstallSnapshotRequest>,
    ) -> Result<Response<proto::InstallSnapshotResponse>, Status> {
        let inner = request.into_inner();
        let raft = self
            .get_raft(&inner.group_id)
            .ok_or_else(|| Status::failed_precondition(format!("group {} 不存在或未启动", inner.group_id)))?;

        let vote: openraft::Vote<NodeId> = bincode::deserialize(&inner.vote)
            .map_err(|e| Status::invalid_argument(format!("Failed to deserialize Vote: {}", e)))?;
        let meta: openraft::SnapshotMeta<NodeId, openraft::BasicNode> = bincode::deserialize(&inner.meta)
            .map_err(|e| Status::invalid_argument(format!("Failed to deserialize SnapshotMeta: {}", e)))?;

        let snapshot = openraft::Snapshot {
            meta,
            snapshot: Box::new(inner.data),
        };

        let resp = raft
            .install_full_snapshot(vote, snapshot)
            .await
            .map_err(|e| Status::internal(format!("InstallSnapshot RPC error: {}", e)))?;

        let vote_bytes = bincode::serialize(&resp.vote)
            .map_err(|e| Status::internal(format!("Failed to serialize Vote: {}", e)))?;
        Ok(Response::new(proto::InstallSnapshotResponse { vote: vote_bytes }))
    }
}

// ── 启动入口 ──

/// 启动 gRPC 服务器。接收一个已调用 `start()` 初始化过的服务实例（Arc 共享，REST 接口同用）。
pub async fn start_raft_service(service: Arc<RaftKvServiceImpl>) -> Result<(), String> {
    let addr = service
        .config
        .address
        .parse()
        .map_err(|e| format!("Invalid address: {}", e))?;

    println!("🚀 Raft 分布式 KV 服务启动中: {}", addr);
    Server::builder()
        // 快照可能较大，放宽 gRPC 帧大小上限。
        // 注意：h2 的 MAX_FRAME_SIZE 上限为 (1<<24)-1 = 16777215（约 16MB），
        // 超过该值会在每次建连时 assert panic，此处取合法最大值。
        .max_frame_size((1 << 24) - 1)
        .add_service(proto::raft_kv_service_server::RaftKvServiceServer::from_arc(service))
        .serve(addr)
        .await
        .map_err(|e| format!("gRPC server error: {}", e))?;

    Ok(())
}
