//! laoflchdb_raft_service - 基于 Raft 的分布式 KV 存储服务
//!
//! 使用 openraft 0.9.25 实现 Raft 共识，通过 RocksDB 持久化日志/元数据，gRPC 通信。

pub mod raft_types;
pub mod state_machine;
pub mod log_store;
pub mod network;

use crate::raft_types::{NodeId, Raft};
use crate::log_store::RaftLogStore;
use crate::network::{GrpcClientPool, GrpcNetworkFactory};
use crate::state_machine::RaftKvStateMachine;
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
    /// 多 Raft 组配置（每个组独立选举、独立日志与状态机，共用同一 gRPC 端口）
    #[serde(default)]
    pub groups: Vec<RaftGroupConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RaftGroupConfig {
    /// 组 ID（同一节点上必须唯一）
    pub group_id: u32,
    /// 本节点在该组中的节点 ID（组内唯一）
    pub node_id: u64,
    /// 该组全部节点的地址（含自身；bootstrap 节点初始化时写入完整成员）
    #[serde(default)]
    pub peers: Vec<PeerConfig>,
    /// 该组是否在本节点自举集群（每组应恰有一个节点为 true，其初始 leader 大概率落在该节点）
    #[serde(default)]
    pub bootstrap: bool,
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

pub struct RaftKvServiceImpl {
    config: RaftServiceConfig,
    /// group_id -> 组运行时。Raft 是 Arc<RaftInner>，本身可 Clone；
    /// 这里用 std RwLock 提供 interior mutability，以便 gRPC service 满足 Sync。
    groups: std::sync::RwLock<std::collections::HashMap<u32, GroupRuntime>>,
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
    pub async fn start(&mut self) -> Result<(), String> {
        if self.config.groups.is_empty() {
            return Err("raft_service.groups 未配置任何 Raft 组".to_string());
        }
        for group_cfg in self.config.groups.clone() {
            self.start_group(&group_cfg).await?;
        }
        Ok(())
    }

    /// 启动单个 Raft 组
    async fn start_group(&mut self, group_cfg: &RaftGroupConfig) -> Result<(), String> {
        let group_id = group_cfg.group_id;
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
        let network_factory = GrpcNetworkFactory::new(self.grpc_pool.clone(), group_id);
        let state_machine = RaftKvStateMachine::new();

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

        // 仅该组的 bootstrap 节点自举集群（自身 + 配置中的全部 peers）；
        // 非 bootstrap 节点不 initialize，保持空日志状态，通过 leader 的日志复制学习成员关系。
        if group_cfg.bootstrap {
            let initialized = raft
                .is_initialized()
                .await
                .map_err(|e| format!("Failed to check initialization: {}", e))?;
            if !initialized {
                let mut members = std::collections::BTreeMap::new();
                members.insert(
                    group_cfg.node_id,
                    openraft::BasicNode { addr: self.config.address.clone() },
                );
                for p in &group_cfg.peers {
                    members.insert(p.id, openraft::BasicNode { addr: p.addr.clone() });
                }
                if let Err(e) = raft.initialize(members).await {
                    // NotAllowed 表示集群已初始化，可安全忽略
                    log::warn!("组 {} 初始化集群失败（可能已初始化）: {}", group_id, e);
                }
            }
        } else {
            log::info!("组 {} bootstrap=false，等待集群 leader 加入", group_id);
        }

        self.groups.write().unwrap().insert(
            group_id,
            GroupRuntime { raft, state_machine },
        );

        log::info!("Raft 组 {} 启动完成 (node_id={})", group_id, group_cfg.node_id);
        Ok(())
    }

    /// 获取指定组的 Raft 引用
    pub fn get_raft(&self, group_id: u32) -> Option<Raft> {
        self.groups.read().unwrap().get(&group_id).map(|g| g.raft.clone())
    }

    /// 获取指定组的共享状态机
    pub fn get_state_machine(&self, group_id: u32) -> Option<RaftKvStateMachine> {
        self.groups.read().unwrap().get(&group_id).map(|g| g.state_machine.clone())
    }

    /// 本节点参与的所有组 ID（已启动）
    pub fn group_ids(&self) -> Vec<u32> {
        self.groups.read().unwrap().keys().copied().collect()
    }

    // ── KV 操作 ──

    async fn kv_put(&self, group_id: u32, table: &str, key: Vec<u8>, value: Vec<u8>) -> Result<bool, String> {
        // 在锁内 clone 出 Raft 句柄，避免 std::sync::RwLockReadGuard 跨 await 持有（非 Send）
        let raft = self
            .get_raft(group_id)
            .ok_or(format!("group {} 不存在或未启动", group_id))?;

        let request = crate::raft_types::Request::Put {
            table: table.to_string(),
            key,
            value,
        };

        let response = raft
            .client_write(request)
            .await
            .map_err(|e| format!("Failed to write: {}", e))?;

        Ok(response.data.success)
    }

    async fn kv_get(&self, group_id: u32, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, String> {
        // 直接从状态机读取（读操作不走 Raft 复制）
        let state_machine = self
            .get_state_machine(group_id)
            .ok_or(format!("group {} 不存在或未启动", group_id))?;
        Ok(state_machine.get(table, key).await)
    }

    async fn kv_delete(&self, group_id: u32, table: &str, key: Vec<u8>) -> Result<bool, String> {
        let raft = self
            .get_raft(group_id)
            .ok_or(format!("group {} 不存在或未启动", group_id))?;

        let request = crate::raft_types::Request::Delete {
            table: table.to_string(),
            key,
        };

        let response = raft
            .client_write(request)
            .await
            .map_err(|e| format!("Failed to delete: {}", e))?;

        Ok(response.data.success)
    }

    async fn kv_scan(&self, group_id: u32, table: &str) -> Result<Vec<(Vec<u8>, Vec<u8>)>, String> {
        let state_machine = self
            .get_state_machine(group_id)
            .ok_or(format!("group {} 不存在或未启动", group_id))?;
        Ok(state_machine.scan(table).await)
    }

    // ── 集群管理 ──

    async fn cluster_add_peer(&self, group_id: u32, id: u64, addr: String) -> Result<bool, String> {
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

    async fn cluster_remove_peer(&self, group_id: u32, id: u64) -> Result<bool, String> {
        let raft = self
            .get_raft(group_id)
            .ok_or(format!("group {} 不存在或未启动", group_id))?;

        let members = std::collections::BTreeSet::from([id]);
        raft.change_membership(openraft::ChangeMembers::RemoveVoters(members), false)
            .await
            .map_err(|e| format!("Failed to remove peer: {}", e))?;

        Ok(true)
    }

    async fn cluster_list_peers(&self, group_id: u32) -> Result<Vec<(u64, String)>, String> {
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

    async fn node_status(&self, group_id: u32) -> Result<String, String> {
        let raft = self
            .get_raft(group_id)
            .ok_or(format!("group {} 不存在或未启动", group_id))?;

        let rx = raft.metrics();
        Ok(format!("{:?}", *rx.borrow()))
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
        match self.kv_put(req.group_id, &req.table, req.key, req.value).await {
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
        match self.kv_get(req.group_id, &req.table, &req.key).await {
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
        match self.kv_delete(req.group_id, &req.table, req.key).await {
            Ok(success) => Ok(Response::new(proto::DeleteResponse { success })),
            Err(e) => Err(Status::internal(e)),
        }
    }

    async fn scan(
        &self,
        request: Request<proto::ScanRequest>,
    ) -> Result<Response<proto::ScanResponse>, Status> {
        let req = request.into_inner();
        match self.kv_scan(req.group_id, &req.table).await {
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
        match self.cluster_add_peer(req.group_id, req.id, req.addr).await {
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
        match self.cluster_remove_peer(req.group_id, req.id).await {
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
        match self.cluster_list_peers(req.group_id).await {
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
        match self.node_status(req.group_id).await {
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
            .get_raft(inner.group_id)
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
            .get_raft(inner.group_id)
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
            .get_raft(inner.group_id)
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

/// 启动 gRPC 服务器。接收一个已调用 `start()` 初始化过的服务实例。
pub async fn start_raft_service(service: RaftKvServiceImpl) -> Result<(), String> {
    let addr = service
        .config
        .address
        .parse()
        .map_err(|e| format!("Invalid address: {}", e))?;

    let svc = Arc::new(service);

    println!("🚀 Raft 分布式 KV 服务启动中: {}", addr);
    Server::builder()
        // 快照可能较大，放宽 gRPC 帧大小上限。
        // 注意：h2 的 MAX_FRAME_SIZE 上限为 (1<<24)-1 = 16777215（约 16MB），
        // 超过该值会在每次建连时 assert panic，此处取合法最大值。
        .max_frame_size((1 << 24) - 1)
        .add_service(proto::raft_kv_service_server::RaftKvServiceServer::from_arc(svc))
        .serve(addr)
        .await
        .map_err(|e| format!("gRPC server error: {}", e))?;

    Ok(())
}
