//! Raft 日志存储实现（基于 RocksDB）
//!
//! 实现 openraft 0.9.25 的 RaftLogStorage 和 RaftLogReader trait。
//! 日志存储在两个 column family 中：
//! - `raft_logs`：日志条目，key = `L` + index(8 字节大端)
//! - `raft_meta`：投票（KEY_VOTE）与已清除日志位置（KEY_LAST_PURGED）

use crate::raft_types::{NodeId, TypeConfig};
use openraft::storage::{LogFlushed, RaftLogReader, RaftLogStorage};
use openraft::{AnyError, Entry, LogId, LogState, OptionalSend, StorageError, StorageIOError, Vote};
use rocksdb::{DB, Options};
use std::fmt::Debug;
use std::ops::RangeBounds;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const CF_LOGS: &str = "raft_logs";
const CF_META: &str = "raft_meta";

/// 日志键前缀
const KEY_LOG_PREFIX: u8 = b'L';
/// 投票键
const KEY_VOTE: &str = "vote";
/// 最后清除日志键
const KEY_LAST_PURGED: &str = "last_purged";

/// Raft 日志存储
#[derive(Clone)]
pub struct RaftLogStore {
    db: Arc<DB>,
    path: PathBuf,
}

impl RaftLogStore {
    pub fn open(path: &Path) -> Result<Self, String> {
        let mut options = Options::default();
        options.create_if_missing(true);
        options.create_missing_column_families(true);

        let cfs = vec![
            rocksdb::ColumnFamilyDescriptor::new(CF_LOGS, Options::default()),
            rocksdb::ColumnFamilyDescriptor::new(CF_META, Options::default()),
        ];

        let db = DB::open_cf_descriptors(&options, path, cfs)
            .map_err(|e| format!("Failed to open RocksDB: {}", e))?;

        Ok(Self {
            db: Arc::new(db),
            path: path.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn logs_cf(&self) -> &rocksdb::ColumnFamily {
        self.db.cf_handle(CF_LOGS).unwrap()
    }

    fn meta_cf(&self) -> &rocksdb::ColumnFamily {
        self.db.cf_handle(CF_META).unwrap()
    }

    /// 日志条目 -> RocksDB key
    fn log_key(index: u64) -> Vec<u8> {
        let mut key = vec![KEY_LOG_PREFIX];
        key.extend_from_slice(&index.to_be_bytes());
        key
    }

    /// 从 RocksDB key 解析日志索引
    fn parse_log_key(key: &[u8]) -> Option<u64> {
        if key.len() == 9 && key[0] == KEY_LOG_PREFIX {
            let index = u64::from_be_bytes(key[1..9].try_into().ok()?);
            Some(index)
        } else {
            None
        }
    }

    /// 获取最后一条日志索引
    fn last_log_index(&self) -> Option<u64> {
        let mut iter = self.db.iterator_cf(self.logs_cf(), rocksdb::IteratorMode::End);
        match iter.next() {
            Some(Ok((key, _value))) => Self::parse_log_key(&key),
            _ => None,
        }
    }

    /// 根据日志条目反序列化
    fn deserialize_entry(blob: &[u8]) -> Option<Entry<TypeConfig>> {
        bincode::deserialize(blob).ok()
    }

    /// 序列化日志条目
    fn serialize_entry(entry: &Entry<TypeConfig>) -> Result<Vec<u8>, String> {
        bincode::serialize(entry).map_err(|e| format!("Failed to serialize entry: {}", e))
    }

    /// 读取某个日志索引处的条目
    fn get_entry(&self, index: u64) -> Option<Entry<TypeConfig>> {
        self.db
            .get_cf(self.logs_cf(), &Self::log_key(index))
            .ok()
            .flatten()
            .and_then(|v| Self::deserialize_entry(&v))
    }

    /// 遍历全部日志条目（按索引升序），回调传入索引与 key 字节
    fn iterate_logs<F>(&self, mut f: F) -> Result<(), StorageError<NodeId>>
    where
        F: FnMut(u64, &[u8]),
    {
        let mut iter = self.db.iterator_cf(self.logs_cf(), rocksdb::IteratorMode::Start);
        for item in iter.by_ref() {
            let (key, _value) = item
                .map_err(|e| StorageError::IO { source: StorageIOError::read_logs(AnyError::error(format!("RocksDB error: {}", e))) })?;
            if let Some(index) = Self::parse_log_key(&key) {
                f(index, &key);
            }
        }
        Ok(())
    }
}

impl RaftLogStorage<TypeConfig> for RaftLogStore {
    type LogReader = RaftLogReaderImpl;

    async fn get_log_state(&mut self) -> Result<LogState<TypeConfig>, StorageError<NodeId>> {
        let last_purged = self
            .db
            .get_cf(self.meta_cf(), KEY_LAST_PURGED)
            .map_err(|e| StorageError::IO { source: StorageIOError::read_logs(AnyError::error(format!("RocksDB error: {}", e))) })?
            .and_then(|v| bincode::deserialize::<LogId<NodeId>>(&v).ok());

        let last_log_id = self.last_log_index().map(|index| {
            self.get_entry(index)
                .map(|e| e.log_id)
                .unwrap_or_else(|| LogId {
                    leader_id: Default::default(),
                    index,
                })
        });

        Ok(LogState {
            last_purged_log_id: last_purged,
            last_log_id,
        })
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        RaftLogReaderImpl {
            db: self.db.clone(),
        }
    }

    async fn save_vote(&mut self, vote: &Vote<NodeId>) -> Result<(), StorageError<NodeId>> {
        let serialized = bincode::serialize(vote)
            .map_err(|e| StorageError::IO { source: StorageIOError::write_vote(AnyError::error(format!("Serialization error: {}", e))) })?;
        self.db
            .put_cf(self.meta_cf(), KEY_VOTE, &serialized)
            .map_err(|e| StorageError::IO { source: StorageIOError::write_vote(AnyError::error(format!("RocksDB error: {}", e))) })?;
        Ok(())
    }

    async fn read_vote(&mut self) -> Result<Option<Vote<NodeId>>, StorageError<NodeId>> {
        let value = self
            .db
            .get_cf(self.meta_cf(), KEY_VOTE)
            .map_err(|e| StorageError::IO { source: StorageIOError::read_vote(AnyError::error(format!("RocksDB error: {}", e))) })?;

        match value {
            Some(blob) => {
                let vote: Vote<NodeId> = bincode::deserialize(&blob)
                    .map_err(|e| StorageError::IO { source: StorageIOError::read_vote(AnyError::error(format!("Serialization error: {}", e))) })?;
                Ok(Some(vote))
            }
            None => Ok(None),
        }
    }

    async fn append<I>(
        &mut self,
        entries: I,
        callback: LogFlushed<TypeConfig>,
    ) -> Result<(), StorageError<NodeId>>
    where
        I: IntoIterator<Item = Entry<TypeConfig>> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        let mut write_batch = rocksdb::WriteBatch::default();
        for entry in entries {
            let index = entry.log_id.index;
            let key = Self::log_key(index);
            let value = Self::serialize_entry(&entry)
                .map_err(|e| StorageError::IO { source: StorageIOError::write_logs(AnyError::error(e)) })?;
            write_batch.put_cf(self.logs_cf(), &key, &value);
        }

        self.db
            .write(&write_batch)
            .map_err(|e| StorageError::IO { source: StorageIOError::write_logs(AnyError::error(format!("RocksDB error: {}", e))) })?;

        // 通知 openraft 日志已持久化
        callback.log_io_completed(Ok(()));

        Ok(())
    }

    /// 删除 log_id 及其之后的所有日志条目（含 log_id 本身）
    async fn truncate(&mut self, log_id: LogId<NodeId>) -> Result<(), StorageError<NodeId>> {
        let mut batch = rocksdb::WriteBatch::default();
        let cf = self.logs_cf();
        self.iterate_logs(|index, key| {
            if index >= log_id.index {
                batch.delete_cf(cf, key);
            }
        })?;
        self.db
            .write(&batch)
            .map_err(|e| StorageError::IO { source: StorageIOError::write_logs(AnyError::error(format!("RocksDB error: {}", e))) })?;
        Ok(())
    }

    /// 删除 log_id 及其之前的所有日志条目（含 log_id 本身），并记录 last_purged
    ///
    /// 安全护栏：只允许清理到“已持久化快照”覆盖的位置，且没有快照时直接跳过。
    /// 日志中的 Membership 条目一旦被删且无快照可恢复，节点重启后该组会退化为
    /// `Learner` + 空成员配置，永远选不出 leader（写入报 `has to forward request to: None, None`）。
    async fn purge(&mut self, log_id: LogId<NodeId>) -> Result<(), StorageError<NodeId>> {
        let snapshot_index = crate::snapshot_store::SnapshotStore::new(&self.path).last_index();
        let purge_to = match snapshot_index {
            Some(idx) if idx >= log_id.index => log_id,
            Some(idx) => LogId {
                leader_id: log_id.leader_id.clone(),
                index: idx,
            },
            None => {
                log::warn!(
                    "组日志目录 {} 尚无持久化快照，跳过 purge(index={})，避免重启后成员关系丢失",
                    self.path.display(),
                    log_id.index
                );
                return Ok(());
            }
        };

        let mut batch = rocksdb::WriteBatch::default();
        let cf = self.logs_cf();
        let purge_index = purge_to.index;
        self.iterate_logs(|index, key| {
            if index <= purge_index {
                batch.delete_cf(cf, key);
            }
        })?;

        let serialized = bincode::serialize(&purge_to)
            .map_err(|e| StorageError::IO { source: StorageIOError::write_logs(AnyError::error(format!("Serialization error: {}", e))) })?;
        batch.put_cf(self.meta_cf(), KEY_LAST_PURGED, &serialized);

        self.db
            .write(&batch)
            .map_err(|e| StorageError::IO { source: StorageIOError::write_logs(AnyError::error(format!("RocksDB error: {}", e))) })?;
        Ok(())
    }
}

/// 日志读取器（由多个复制任务并发使用）
pub struct RaftLogReaderImpl {
    db: Arc<DB>,
}

impl RaftLogReader<TypeConfig> for RaftLogReaderImpl {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry<TypeConfig>>, StorageError<NodeId>> {
        let mut entries = Vec::new();
        let mut iter = self.db.iterator_cf(self.db.cf_handle(CF_LOGS).unwrap(), rocksdb::IteratorMode::Start);
        for item in iter.by_ref() {
            let (key, value) = item
                .map_err(|e| StorageError::IO { source: StorageIOError::read_logs(AnyError::error(format!("RocksDB error: {}", e))) })?;
            if let Some(index) = RaftLogStore::parse_log_key(&key) {
                if range.contains(&index) {
                    if let Some(entry) = RaftLogStore::deserialize_entry(&value) {
                        entries.push(entry);
                    }
                }
            }
        }
        Ok(entries)
    }
}

// RaftLogStorage 以 RaftLogReader 为 supertrait，因此 RaftLogStore 自身也要实现读取接口。
impl RaftLogReader<TypeConfig> for RaftLogStore {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry<TypeConfig>>, StorageError<NodeId>> {
        let mut entries = Vec::new();
        let mut iter = self.db.iterator_cf(self.logs_cf(), rocksdb::IteratorMode::Start);
        for item in iter.by_ref() {
            let (key, value) = item
                .map_err(|e| StorageError::IO { source: StorageIOError::read_logs(AnyError::error(format!("RocksDB error: {}", e))) })?;
            if let Some(index) = Self::parse_log_key(&key) {
                if range.contains(&index) {
                    if let Some(entry) = Self::deserialize_entry(&value) {
                        entries.push(entry);
                    }
                }
            }
        }
        Ok(entries)
    }
}
