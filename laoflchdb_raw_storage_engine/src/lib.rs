//! laoflchdb_raw_storage_engine
//!
//! 原始分块存储引擎：把 chunk 数据块直接写入文件系统中的一个文件。
//!
//! # 文件布局
//!
//! `{root}/{id[0..2]}/{id[2..4]}/{id}.chunk`（分片层级由 `shard_depth` 控制，默认 2）
//!
//! # 文件格式
//!
//! `[magic "LCHK" 4B][sha256 32B][data_len u64 LE][data]`
//!
//! 写入采用「临时文件 + fsync + 原子 rename」，读取时校验 sha256。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// 文件头长度：magic(4) + sha256(32) + len(8)
pub const HEADER_LEN: usize = 44;
const MAGIC: &[u8; 4] = b"LCHK";

/// 原始分块存储配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawStorageConfig {
    /// 数据根目录
    #[serde(default = "default_root")]
    pub root: String,
    /// 写入后是否执行 fsync
    #[serde(default = "default_fsync")]
    pub fsync: bool,
    /// 目录分片层级（每层 2 个字符）
    #[serde(default = "default_shard_depth")]
    pub shard_depth: usize,
}

fn default_root() -> String {
    "./laoflch_raw_store".to_string()
}
fn default_fsync() -> bool {
    true
}
fn default_shard_depth() -> usize {
    2
}

impl Default for RawStorageConfig {
    fn default() -> Self {
        Self {
            root: default_root(),
            fsync: default_fsync(),
            shard_depth: default_shard_depth(),
        }
    }
}

/// chunk 元信息
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkStat {
    pub size: u64,
    pub sha256: [u8; 32],
}

/// 分块存储抽象
#[async_trait]
pub trait ChunkStorage: Send + Sync + 'static {
    /// 引擎类型名
    fn engine_type(&self) -> &str;

    /// 写入一个 chunk（同 id 覆盖写）
    async fn put_chunk(
        &self,
        chunk_id: &str,
        data: &[u8],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// 读取一个 chunk（不存在返回 None）
    async fn get_chunk(
        &self,
        chunk_id: &str,
    ) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>>;

    /// 删除一个 chunk（不存在视为成功）
    async fn delete_chunk(
        &self,
        chunk_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// 查询 chunk 元信息（不存在返回 None）
    async fn stat_chunk(
        &self,
        chunk_id: &str,
    ) -> Result<Option<ChunkStat>, Box<dyn std::error::Error + Send + Sync>>;

    /// 健康检查
    async fn health(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

/// 基于文件系统的原始分块存储引擎
pub struct RawStorageEngine {
    root: PathBuf,
    fsync: bool,
    shard_depth: usize,
}

impl RawStorageEngine {
    pub fn new(config: &RawStorageConfig) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let root = PathBuf::from(&config.root);
        std::fs::create_dir_all(&root)?;
        Ok(Self {
            root,
            fsync: config.fsync,
            shard_depth: config.shard_depth.max(1),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 校验 chunk_id，避免路径穿越
    fn validate_id(chunk_id: &str) -> Result<(), String> {
        if chunk_id.is_empty() || chunk_id.len() > 128 {
            return Err(format!("chunk_id 长度非法: {:?}", chunk_id));
        }
        if !chunk_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(format!("chunk_id 含非法字符: {:?}", chunk_id));
        }
        Ok(())
    }

    /// chunk 文件路径
    pub fn path_for(&self, chunk_id: &str) -> Result<PathBuf, String> {
        Self::validate_id(chunk_id)?;
        let mut path = self.root.clone();
        for level in 0..self.shard_depth {
            let start = level * 2;
            if start + 2 <= chunk_id.len() {
                path.push(&chunk_id[start..start + 2]);
            } else {
                path.push("00");
            }
        }
        path.push(format!("{}.chunk", chunk_id));
        Ok(path)
    }

    async fn read_header(path: &Path) -> Result<Option<ChunkStat>, Box<dyn std::error::Error + Send + Sync>> {
        use tokio::io::AsyncReadExt;
        let mut file = match tokio::fs::File::open(path).await {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(Box::new(e)),
        };
        let mut header = [0u8; HEADER_LEN];
        if let Err(e) = file.read_exact(&mut header).await {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                return Err("chunk 文件头不完整".into());
            }
            return Err(Box::new(e));
        }
        if &header[0..4] != MAGIC {
            return Err("chunk 文件魔数不匹配".into());
        }
        let mut sha256 = [0u8; 32];
        sha256.copy_from_slice(&header[4..36]);
        let size = u64::from_le_bytes(header[36..44].try_into().unwrap());
        Ok(Some(ChunkStat { size, sha256 }))
    }
}

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let out = hasher.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out);
    arr
}

#[async_trait]
impl ChunkStorage for RawStorageEngine {
    fn engine_type(&self) -> &str {
        "raw_file"
    }

    async fn put_chunk(
        &self,
        chunk_id: &str,
        data: &[u8],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use tokio::io::AsyncWriteExt;

        let path = self.path_for(chunk_id).map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
        let parent = path
            .parent()
            .ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> { "chunk 路径无父目录".into() })?;
        tokio::fs::create_dir_all(parent).await?;

        let digest = sha256(data);
        let tmp_path = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4().simple()));

        let write_result: Result<(), Box<dyn std::error::Error + Send + Sync>> = async {
            let mut file = tokio::fs::File::create(&tmp_path).await?;
            let mut header = Vec::with_capacity(HEADER_LEN);
            header.extend_from_slice(MAGIC);
            header.extend_from_slice(&digest);
            header.extend_from_slice(&(data.len() as u64).to_le_bytes());
            file.write_all(&header).await?;
            file.write_all(data).await?;
            if self.fsync {
                file.sync_all().await?;
            }
            drop(file);
            tokio::fs::rename(&tmp_path, &path).await?;
            if self.fsync {
                // 尽力 sync 目录项，失败不致命
                if let Ok(dir) = tokio::fs::File::open(parent).await {
                    let _ = dir.sync_all().await;
                }
            }
            Ok(())
        }
        .await;

        if write_result.is_err() {
            let _ = tokio::fs::remove_file(&tmp_path).await;
        }
        write_result
    }

    async fn get_chunk(
        &self,
        chunk_id: &str,
    ) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        use tokio::io::AsyncReadExt;

        let path = self.path_for(chunk_id).map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
        let mut file = match tokio::fs::File::open(&path).await {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(Box::new(e)),
        };

        let mut header = [0u8; HEADER_LEN];
        file.read_exact(&mut header).await?;
        if &header[0..4] != MAGIC {
            return Err("chunk 文件魔数不匹配".into());
        }
        let mut expected = [0u8; 32];
        expected.copy_from_slice(&header[4..36]);
        let len = u64::from_le_bytes(header[36..44].try_into().unwrap()) as usize;

        let mut data = vec![0u8; len];
        file.read_exact(&mut data).await?;

        if sha256(&data) != expected {
            return Err(format!("chunk {} 校验失败", chunk_id).into());
        }
        Ok(Some(data))
    }

    async fn delete_chunk(
        &self,
        chunk_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let path = self.path_for(chunk_id).map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Box::new(e)),
        }
    }

    async fn stat_chunk(
        &self,
        chunk_id: &str,
    ) -> Result<Option<ChunkStat>, Box<dyn std::error::Error + Send + Sync>> {
        let path = self.path_for(chunk_id).map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
        Self::read_header(&path).await
    }

    async fn health(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        tokio::fs::metadata(&self.root).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn put_get_delete_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let engine = RawStorageEngine::new(&RawStorageConfig {
            root: dir.path().to_string_lossy().into_owned(),
            fsync: false,
            shard_depth: 2,
        })
        .unwrap();

        let data = b"hello raw storage";
        engine.put_chunk("abcdef123456", data).await.unwrap();
        let got = engine.get_chunk("abcdef123456").await.unwrap().unwrap();
        assert_eq!(got, data);

        let stat = engine.stat_chunk("abcdef123456").await.unwrap().unwrap();
        assert_eq!(stat.size, data.len() as u64);
        assert_eq!(stat.sha256, sha256(data));

        engine.delete_chunk("abcdef123456").await.unwrap();
        assert!(engine.get_chunk("abcdef123456").await.unwrap().is_none());
    }

    #[test]
    fn rejects_path_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let engine = RawStorageEngine::new(&RawStorageConfig {
            root: dir.path().to_string_lossy().into_owned(),
            fsync: false,
            shard_depth: 2,
        })
        .unwrap();
        assert!(engine.path_for("../etc/passwd").is_err());
    }
}
