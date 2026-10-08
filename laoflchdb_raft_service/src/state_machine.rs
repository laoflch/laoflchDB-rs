//! Raft 状态机实现
//!
//! 基于内存的 KV 存储，通过 openraft 0.9.25 的 RaftStateMachine trait 实现。
//! 内部使用 Arc<RwLock> 共享底层状态，RaftKvStateMachine 可 Clone 后分别
//! 提供给 Raft 核心与 gRPC 读接口使用。

use crate::raft_types::{NodeId, Request, Response, TypeConfig};
use openraft::storage::{RaftSnapshotBuilder, RaftStateMachine, Snapshot};
use openraft::{
    BasicNode, EntryPayload, LogId, OptionalSend, SnapshotMeta, StorageError, StoredMembership,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// KV 存储状态机
#[derive(Clone, Default)]
pub struct RaftKvStateMachine {
    /// 表 -> key -> value 的内存存储
    tables: Arc<RwLock<HashMap<String, HashMap<Vec<u8>, Vec<u8>>>>>,
    /// 最后应用的日志 ID
    last_applied: Arc<RwLock<Option<LogId<NodeId>>>>,
    /// 当前成员配置
    membership: Arc<RwLock<StoredMembership<NodeId, BasicNode>>>,
}

impl RaftKvStateMachine {
    pub fn new() -> Self {
        Self {
            tables: Arc::new(RwLock::new(HashMap::new())),
            last_applied: Arc::new(RwLock::new(None)),
            membership: Arc::new(RwLock::new(StoredMembership::new(None, openraft::Membership::default()))),
        }
    }

    /// 获取数据
    pub async fn get(&self, table: &str, key: &[u8]) -> Option<Vec<u8>> {
        let tables = self.tables.read().await;
        tables.get(table).and_then(|m| m.get(key)).cloned()
    }

    /// 扫描表中所有键值对
    pub async fn scan(&self, table: &str) -> Vec<(Vec<u8>, Vec<u8>)> {
        let tables = self.tables.read().await;
        tables
            .get(table)
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default()
    }

    /// 序列化状态为快照数据
    async fn serialize_state(&self) -> Vec<u8> {
        let tables = self.tables.read().await;
        bincode::serialize(&*tables).unwrap_or_default()
    }

    /// 从快照数据恢复状态
    async fn deserialize_state(&self, data: &[u8]) {
        if let Ok(table_data) = bincode::deserialize::<HashMap<String, HashMap<Vec<u8>, Vec<u8>>>>(data) {
            let mut tables = self.tables.write().await;
            *tables = table_data;
        }
    }
}

/// 快照构建器
pub struct KvSnapshotBuilder {
    state_machine: RaftKvStateMachine,
}

impl RaftSnapshotBuilder<TypeConfig> for KvSnapshotBuilder {
    async fn build_snapshot(&mut self) -> Result<Snapshot<TypeConfig>, StorageError<NodeId>> {
        let data = self.state_machine.serialize_state().await;

        // 获取当前状态
        let last_applied = *self.state_machine.last_applied.read().await;
        let membership = self.state_machine.membership.read().await.clone();

        let (term, index) = last_applied
            .map(|l| (l.leader_id.term, l.index))
            .unwrap_or((0, 0));

        let meta = SnapshotMeta {
            last_log_id: last_applied,
            last_membership: membership,
            snapshot_id: format!("{}-{}-{}.snap", term, index, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)),
        };

        Ok(Snapshot {
            meta,
            snapshot: Box::new(data),
        })
    }
}

impl RaftStateMachine<TypeConfig> for RaftKvStateMachine {
    type SnapshotBuilder = KvSnapshotBuilder;

    async fn applied_state(
        &mut self,
    ) -> Result<(Option<LogId<NodeId>>, StoredMembership<NodeId, BasicNode>), StorageError<NodeId>> {
        let last_applied = *self.last_applied.read().await;
        let membership = self.membership.read().await.clone();
        Ok((last_applied, membership))
    }

    async fn apply<I>(&mut self, entries: I) -> Result<Vec<Response>, StorageError<NodeId>>
    where
        I: IntoIterator<Item = openraft::Entry<TypeConfig>> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        // 注意：返回的响应数量必须与条目数量一致（Blank/Membership 条目也要返回一个响应）
        let mut responses = Vec::new();
        for entry in entries {
            let log_id = entry.log_id;
            let response = match &entry.payload {
                EntryPayload::Normal(req) => match req {
                    Request::Put { table, key, value } => {
                        self.tables
                            .write()
                            .await
                            .entry(table.clone())
                            .or_default()
                            .insert(key.clone(), value.clone());
                        Response { success: true, message: "OK".to_string() }
                    }
                    Request::Delete { table, key } => {
                        if let Some(tbl) = self.tables.write().await.get_mut(table) {
                            tbl.remove(key);
                        }
                        Response { success: true, message: "OK".to_string() }
                    }
                },
                EntryPayload::Membership(m) => {
                    *self.membership.write().await = StoredMembership::new(Some(log_id), m.clone());
                    Response { success: true, message: "membership changed".to_string() }
                }
                EntryPayload::Blank => {
                    Response { success: true, message: "blank".to_string() }
                }
            };

            *self.last_applied.write().await = Some(log_id);
            responses.push(response);
        }
        Ok(responses)
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        KvSnapshotBuilder {
            state_machine: self.clone(),
        }
    }

    async fn begin_receiving_snapshot(&mut self) -> Result<Box<Vec<u8>>, StorageError<NodeId>> {
        // 返回一个空的 Vec 用于接收快照数据
        Ok(Box::new(Vec::new()))
    }

    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<NodeId, BasicNode>,
        snapshot: Box<Vec<u8>>,
    ) -> Result<(), StorageError<NodeId>> {
        self.deserialize_state(&snapshot).await;
        *self.last_applied.write().await = meta.last_log_id;
        *self.membership.write().await = meta.last_membership.clone();
        Ok(())
    }

    async fn get_current_snapshot(
        &mut self,
    ) -> Result<Option<Snapshot<TypeConfig>>, StorageError<NodeId>> {
        let mut builder = self.get_snapshot_builder().await;
        let snapshot = builder.build_snapshot().await?;
        Ok(Some(snapshot))
    }
}
