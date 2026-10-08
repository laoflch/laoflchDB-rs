//! Raft 网络层实现（基于 gRPC）
//!
//! 实现 openraft 0.9.25 的 RaftNetworkFactory 和 RaftNetwork trait，
//! 使用 gRPC 进行节点间的 vote、append_entries、full_snapshot 通信。
//! 消息体统一使用 bincode 序列化 openraft 的消息类型。

use crate::raft_types::{NodeId, TypeConfig};
use crate::proto;
use openraft::error::{Fatal, NetworkError, RPCError, ReplicationClosed, StreamingError};
use openraft::network::{RPCOption, RaftNetwork, RaftNetworkFactory};
use openraft::raft::AppendEntriesRequest;
use openraft::{BasicNode, OptionalSend, Snapshot, Vote};
use tonic::transport::Channel;

/// gRPC 客户端连接池
#[derive(Clone)]
pub struct GrpcClientPool {
    clients: std::sync::Arc<tokio::sync::RwLock<std::collections::HashMap<String, proto::raft_kv_service_client::RaftKvServiceClient<Channel>>>>,
}

impl GrpcClientPool {
    pub fn new() -> Self {
        Self {
            clients: std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        }
    }

    pub async fn get_client(
        &self,
        addr: &str,
    ) -> Result<proto::raft_kv_service_client::RaftKvServiceClient<Channel>, String> {
        let mut clients = self.clients.write().await;
        if let Some(client) = clients.get(addr).cloned() {
            return Ok(client);
        }

        let url = if addr.starts_with("http://") || addr.starts_with("https://") {
            addr.to_string()
        } else {
            format!("http://{}", addr)
        };

        let channel = Channel::from_shared(url)
            .map_err(|e| format!("Invalid address: {}", e))?
            // 放宽 HTTP/2 流窗口，以支持较大的 raft 消息/快照传输
            // （客户端帧大小由 h2 协商决定，tonic 0.11 的 Endpoint 无 max_frame_size 方法）
            .initial_stream_window_size(16 * 1024 * 1024)
            .initial_connection_window_size(16 * 1024 * 1024)
            .connect()
            .await
            .map_err(|e| format!("Failed to connect: {}", e))?;

        let client = proto::raft_kv_service_client::RaftKvServiceClient::new(channel);
        clients.insert(addr.to_string(), client.clone());
        Ok(client)
    }
}

impl Default for GrpcClientPool {
    fn default() -> Self {
        Self::new()
    }
}

/// 把字符串错误包装成 NetworkError
fn network_error(msg: String) -> NetworkError {
    let io_err = std::io::Error::new(std::io::ErrorKind::Other, msg);
    NetworkError::new(&io_err)
}

/// Raft 网络工厂（每个 Raft 组一份，携带本组 group_id 用于内部 RPC 路由）
pub struct GrpcNetworkFactory {
    pool: GrpcClientPool,
    group_id: u32,
}

impl GrpcNetworkFactory {
    pub fn new(pool: GrpcClientPool, group_id: u32) -> Self {
        Self { pool, group_id }
    }
}

impl RaftNetworkFactory<TypeConfig> for GrpcNetworkFactory {
    type Network = GrpcNetworkClient;

    async fn new_client(&mut self, target: NodeId, node: &BasicNode) -> Self::Network {
        GrpcNetworkClient {
            target,
            node_addr: node.addr.clone(),
            pool: self.pool.clone(),
            group_id: self.group_id,
        }
    }
}

/// Raft 网络客户端（负责与单个目标节点通信）
pub struct GrpcNetworkClient {
    target: NodeId,
    node_addr: String,
    pool: GrpcClientPool,
    group_id: u32,
}

impl RaftNetwork<TypeConfig> for GrpcNetworkClient {
    async fn vote(
        &mut self,
        rpc: openraft::raft::VoteRequest<NodeId>,
        _option: RPCOption,
    ) -> Result<openraft::raft::VoteResponse<NodeId>, RPCError<NodeId, BasicNode, openraft::error::RaftError<NodeId>>> {
        let client = self
            .pool
            .get_client(&self.node_addr)
            .await
            .map_err(network_error)?;

        let payload = bincode::serialize(&rpc)
            .map_err(|e| network_error(format!("Failed to serialize VoteRequest: {}", e)))?;

        let request = tonic::Request::new(proto::RequestVoteRequest {
            group_id: self.group_id,
            payload,
        });
        let resp = client
            .clone()
            .request_vote(request)
            .await
            .map_err(|e| network_error(format!("RequestVote RPC failed: {}", e)))?;

        let inner = resp.into_inner();
        bincode::deserialize(&inner.payload)
            .map_err(|e| network_error(format!("Failed to deserialize VoteResponse: {}", e)))?
    }

    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<openraft::raft::AppendEntriesResponse<NodeId>, RPCError<NodeId, BasicNode, openraft::error::RaftError<NodeId>>> {
        let client = self
            .pool
            .get_client(&self.node_addr)
            .await
            .map_err(network_error)?;

        let payload = bincode::serialize(&rpc)
            .map_err(|e| network_error(format!("Failed to serialize AppendEntriesRequest: {}", e)))?;

        log::info!(
            "[raft-net] append_entries -> {} req_payload_len={}",
            self.node_addr,
            payload.len()
        );
        let request = tonic::Request::new(proto::AppendEntriesRequest {
            group_id: self.group_id,
            payload,
        });
        let resp = client
            .clone()
            .append_entries(request)
            .await
            .map_err(|e| network_error(format!("AppendEntries RPC failed: {}", e)))?;

        let inner = resp.into_inner();
        log::info!(
            "[raft-net] append_entries <- {} resp_payload_len={}",
            self.node_addr,
            inner.payload.len()
        );
        // 显式类型标注，排除泛型推断问题
        let resp_de: openraft::raft::AppendEntriesResponse<NodeId> =
            bincode::deserialize(&inner.payload)
                .map_err(|e| {
                    // 取证：payload 应等于 Fixint u32(0) 的 4 字节；若 u32 解码失败则格式异常
                    let u32_de: Result<u32, _> = bincode::deserialize(&inner.payload);
                    log::error!(
                        "[raft-net] append_entries resp deserialize FAILED len={} hex={:02x?} u32_de={:?} err={:?}",
                        inner.payload.len(),
                        &inner.payload[..inner.payload.len().min(96)],
                        u32_de,
                        e
                    );
                    network_error(format!("Failed to deserialize AppendEntriesResponse: {}", e))
                })?;
        Ok(resp_de)
    }

    async fn full_snapshot(
        &mut self,
        vote: Vote<NodeId>,
        snapshot: Snapshot<TypeConfig>,
        _cancel: impl std::future::Future<Output = ReplicationClosed> + OptionalSend + 'static,
        _option: RPCOption,
    ) -> Result<openraft::raft::SnapshotResponse<NodeId>, StreamingError<TypeConfig, Fatal<NodeId>>> {
        let client = self
            .pool
            .get_client(&self.node_addr)
            .await
            .map_err(|e| StreamingError::Network(network_error(e)))?;

        let vote_bytes = bincode::serialize(&vote)
            .map_err(|e| StreamingError::Network(network_error(format!("Failed to serialize Vote: {}", e))))?;
        let meta_bytes = bincode::serialize(&snapshot.meta)
            .map_err(|e| StreamingError::Network(network_error(format!("Failed to serialize SnapshotMeta: {}", e))))?;

        let request = tonic::Request::new(proto::InstallSnapshotRequest {
            group_id: self.group_id,
            vote: vote_bytes,
            meta: meta_bytes,
            offset: 0,
            data: *snapshot.snapshot,
            done: true,
        });

        let resp = client
            .clone()
            .install_snapshot(request)
            .await
            .map_err(|e| StreamingError::Network(network_error(format!("InstallSnapshot RPC failed: {}", e))))?;

        let inner = resp.into_inner();
        let resp_vote: Vote<NodeId> = bincode::deserialize(&inner.vote)
            .map_err(|e| StreamingError::Network(network_error(format!("Failed to deserialize Vote: {}", e))))?;

        Ok(openraft::raft::SnapshotResponse::new(resp_vote))
    }
}
