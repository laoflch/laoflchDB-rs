//! Raft 状态机实现
//!
//! 状态机数据以 `KvStorage` 存储引擎为唯一真身（默认 KVRocksDBEngine/rocksdb），
//! 不再维护内存副本。可选"双写"：把业务数据同步写入另一个 KvStorage 引擎，
//! 作为业务数据副本（与 Raft 复制解耦，供业务侧独立读取）。
//! 快照以 KV 流格式构建（iterate 引擎全量 KV → 流式 dump），install 时逐块写回。
//!
//! 内部使用 Arc<RwLock> 共享状态，RaftKvStateMachine 可 Clone 后分别
//! 提供给 Raft 核心与 gRPC 读接口使用。

use crate::raft_types::{NodeId, Request, Response, TypeConfig};
use crate::storage::business_table_name;
use laoflchdb_engines::KvStorage;
use openraft::storage::{RaftSnapshotBuilder, RaftStateMachine, Snapshot};
use openraft::{
    AnyError, BasicNode, EntryPayload, LogId, OptionalSend, SnapshotMeta, StorageError,
    StorageIOError, StoredMembership,
};
use std::sync::Arc;
use tokio::sync::RwLock;

fn storage_io_err(msg: String) -> StorageError<NodeId> {
    StorageError::IO {
        source: StorageIOError::write_logs(AnyError::error(msg)),
    }
}

/// 快照 KV 流格式（可流式追加/逐块读取）：
/// `[u64 count][u64 klen][key][u64 vlen][value]...`，小端。
/// 构建侧 iterate 引擎全量 KV 追加写出；install 侧逐块解析后写回。
fn serialize_kv_stream(entries: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + entries.iter().map(|(k, v)| 16 + k.len() + v.len()).sum::<usize>());
    out.extend_from_slice(&(entries.len() as u64).to_le_bytes());
    for (k, v) in entries {
        out.extend_from_slice(&(k.len() as u64).to_le_bytes());
        out.extend_from_slice(k);
        out.extend_from_slice(&(v.len() as u64).to_le_bytes());
        out.extend_from_slice(v);
    }
    out
}

fn deserialize_kv_stream(data: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
    fn read_u64(data: &[u8], pos: &mut usize) -> Option<u64> {
        if *pos + 8 > data.len() {
            return None;
        }
        let v = u64::from_le_bytes(data[*pos..*pos + 8].try_into().ok()?);
        *pos += 8;
        Some(v)
    }
    let mut out = Vec::new();
    let mut pos = 0usize;
    let Some(count) = read_u64(data, &mut pos) else {
        return out;
    };
    for _ in 0..count {
        let Some(klen) = read_u64(data, &mut pos) else { break };
        let Some(k) = data.get(pos..pos + klen as usize) else { break };
        let key = k.to_vec();
        pos += klen as usize;
        let Some(vlen) = read_u64(data, &mut pos) else { break };
        let Some(v) = data.get(pos..pos + vlen as usize) else { break };
        let value = v.to_vec();
        pos += vlen as usize;
        out.push((key, value));
    }
    out
}

/// KV 状态机：数据以 KvStorage 引擎为唯一真身
#[derive(Clone)]
pub struct RaftKvStateMachine {
    /// 最后应用的日志 ID
    last_applied: Arc<RwLock<Option<LogId<NodeId>>>>,
    /// 当前成员配置
    membership: Arc<RwLock<StoredMembership<NodeId, BasicNode>>>,
    /// 所属 Raft 组（决定业务表名 group_{group_id}）
    group_id: u32,
    /// 状态机真身存储（固定通过 KvStorage，默认 KVRocksDBEngine/rocksdb）
    kvdb: Arc<dyn KvStorage>,
    /// 可选业务数据副本（双写；None 表示未开启双写）
    dual_write: Option<Arc<dyn KvStorage>>,
}

impl RaftKvStateMachine {
    pub fn new(
        group_id: u32,
        kvdb: Arc<dyn KvStorage>,
        dual_write: Option<Arc<dyn KvStorage>>,
    ) -> Self {
        Self {
            last_applied: Arc::new(RwLock::new(None)),
            membership: Arc::new(RwLock::new(StoredMembership::new(None, openraft::Membership::default()))),
            group_id,
            kvdb,
            dual_write,
        }
    }

    /// 当前组的业务表名
    fn business_table(&self) -> String {
        business_table_name(self.group_id)
    }

    /// 写入主引擎；开启双写时同步写入副本
    async fn put_kv(&self, key: &[u8], value: &[u8]) -> Result<(), StorageError<NodeId>> {
        let table = self.business_table();
        self.kvdb
            .put(&table, key, value)
            .await
            .map_err(|e| storage_io_err(format!("状态机 KV 写入失败: {}", e)))?;
        if let Some(dual) = &self.dual_write {
            dual
                .put(&table, key, value)
                .await
                .map_err(|e| storage_io_err(format!("双写副本 KV 写入失败: {}", e)))?;
        }
        Ok(())
    }

    /// 删除主引擎；开启双写时同步删除副本
    async fn delete_kv(&self, key: &[u8]) -> Result<(), StorageError<NodeId>> {
        let table = self.business_table();
        self.kvdb
            .delete(&table, key)
            .await
            .map_err(|e| storage_io_err(format!("状态机 KV 删除失败: {}", e)))?;
        if let Some(dual) = &self.dual_write {
            dual
                .delete(&table, key)
                .await
                .map_err(|e| storage_io_err(format!("双写副本 KV 删除失败: {}", e)))?;
        }
        Ok(())
    }

    /// 获取数据（直查主引擎）
    pub async fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, String> {
        let table = self.business_table();
        self.kvdb
            .get(&table, key)
            .await
            .map_err(|e| format!("状态机 KV 读取失败: {}", e))
    }

    /// 扫描当前组全部键值对（直查主引擎）
    pub async fn scan(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>, String> {
        let table = self.business_table();
        self.kvdb
            .scan(&table)
            .await
            .map_err(|e| format!("状态机 KV 扫描失败: {}", e))
    }

    /// 序列化状态为快照数据（iterate 引擎全量 KV → KV 流）
    async fn serialize_state(&self) -> Result<Vec<u8>, StorageError<NodeId>> {
        let table = self.business_table();
        let entries = self
            .kvdb
            .scan(&table)
            .await
            .map_err(|e| storage_io_err(format!("快照 iterate 失败: {}", e)))?;
        Ok(serialize_kv_stream(&entries))
    }

    /// 从快照 KV 流恢复状态：清空主引擎与副本，逐块解析并写回
    async fn deserialize_state(&self, data: &[u8]) -> Result<(), StorageError<NodeId>> {
        let table = self.business_table();
        self.kvdb
            .clear(&table)
            .await
            .map_err(|e| storage_io_err(format!("状态机清空失败: {}", e)))?;
        if let Some(dual) = &self.dual_write {
            dual
                .clear(&table)
                .await
                .map_err(|e| storage_io_err(format!("双写副本清空失败: {}", e)))?;
        }
        for (k, v) in deserialize_kv_stream(data) {
            self.put_kv(&k, &v).await?;
        }
        Ok(())
    }
}

/// 快照构建器
pub struct KvSnapshotBuilder {
    state_machine: RaftKvStateMachine,
}

impl RaftSnapshotBuilder<TypeConfig> for KvSnapshotBuilder {
    async fn build_snapshot(&mut self) -> Result<Snapshot<TypeConfig>, StorageError<NodeId>> {
        let data = self.state_machine.serialize_state().await?;

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
                    Request::Put { key, value } => {
                        self.put_kv(&key, &value).await?;
                        Response { success: true, message: "OK".to_string() }
                    }
                    Request::Delete { key } => {
                        self.delete_kv(&key).await?;
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
        self.deserialize_state(&snapshot).await?;
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
