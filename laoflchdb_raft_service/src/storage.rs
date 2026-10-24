//! Raft 业务数据落库抽象
//!
//! 通过 `KvStorage` 接口解耦具体存储引擎：配置 `raft_service.storage` 的
//! `type` 字段选择引擎（"rocksdb" 使用 KVRocksDBEngine、"memory" 使用内存实现）。
//! table 概念对应 Raft 组：每组数据存放于 table = "group_{group_id}"。

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use laoflchdb_engines::KvStorage;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// 存储引擎配置（每个 Raft 组共享同一个引擎实例，table 区分组）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RaftStorageConfig {
    /// 引擎类型："rocksdb"（默认，落盘）或 "memory"（纯内存，重启数据丢失）
    #[serde(default = "default_storage_type")]
    pub storage_type: String,
    /// RocksDB 引擎的数据路径（缺省使用 raft_service.data_path/business.db）
    #[serde(default)]
    pub db_path: Option<String>,
    /// 存储 schema 名（RocksDB 引擎元数据用；缺省 "raft"）
    #[serde(default = "default_schema_name")]
    pub schema_name: String,
}

fn default_storage_type() -> String {
    "rocksdb".to_string()
}

fn default_schema_name() -> String {
    "raft".to_string()
}

impl Default for RaftStorageConfig {
    fn default() -> Self {
        Self {
            storage_type: default_storage_type(),
            db_path: None,
            schema_name: default_schema_name(),
        }
    }
}

/// 内存 KV 存储引擎（实现 KvStorage；数据不持久化，重启后从 Raft 日志重放）
#[derive(Default)]
pub struct MemoryKvStorage {
    /// table -> (key -> value)
    tables: RwLock<HashMap<String, HashMap<Vec<u8>, Vec<u8>>>>,
}

impl MemoryKvStorage {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl KvStorage for MemoryKvStorage {
    fn engine_type(&self) -> &str {
        "memory"
    }

    async fn ensure_table(&self, table: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut tables = self.tables.write().await;
        tables.entry(table.to_string()).or_default();
        Ok(())
    }

    async fn put(&self, table: &str, key: &[u8], value: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut tables = self.tables.write().await;
        let t = tables.entry(table.to_string()).or_default();
        t.insert(key.to_vec(), value.to_vec());
        Ok(())
    }

    async fn get(&self, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        let tables = self.tables.read().await;
        Ok(tables.get(table).and_then(|t| t.get(key).cloned()))
    }

    async fn delete(&self, table: &str, key: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut tables = self.tables.write().await;
        if let Some(t) = tables.get_mut(table) {
            t.remove(key);
        }
        Ok(())
    }

    async fn scan(&self, table: &str) -> Result<Vec<(Vec<u8>, Vec<u8>)>, Box<dyn std::error::Error + Send + Sync>> {
        let tables = self.tables.read().await;
        Ok(tables
            .get(table)
            .map(|t| t.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default())
    }

    async fn clear(&self, table: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut tables = self.tables.write().await;
        if let Some(t) = tables.get_mut(table) {
            t.clear();
        }
        Ok(())
    }
}

/// 业务库表名：每个 Raft 组一个表
pub fn business_table_name(group_id: &str) -> String {
    format!("group_{}", group_id)
}

/// 根据配置打开存储引擎（默认 rocksdb，db_path 缺省为 data_path/business.db）
pub fn open_storage(
    config: &RaftStorageConfig,
    default_path: &Path,
) -> Result<Arc<dyn KvStorage>, String> {
    match config.storage_type.as_str() {
        "memory" => {
            log::info!("[raft-storage] 使用内存存储引擎 (memory)");
            Ok(Arc::new(MemoryKvStorage::new()))
        }
        "rocksdb" => {
            let db_path = config
                .db_path
                .clone()
                .unwrap_or_else(|| default_path.to_string_lossy().into_owned());
            let options = laoflchdb_engines::EngineOptions {
                db_path: db_path.clone(),
                schema_name: config.schema_name.clone(),
            };
            log::info!("[raft-storage] 使用 RocksDB 存储引擎, db_path={}", db_path);
            let engine = laoflchdb_kv_rocksdb_engine::KVRocksDBEngine::new(&options)
                .map_err(|e| format!("打开 RocksDB 存储引擎失败 ({}): {}", db_path, e))?;
            Ok(Arc::new(engine))
        }
        other => Err(format!("未知存储引擎类型: {}（可选: rocksdb / memory）", other)),
    }
}
