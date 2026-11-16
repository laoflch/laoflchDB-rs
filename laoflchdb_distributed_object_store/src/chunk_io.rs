//! 对象分块写入 / 流式读取

use crate::error::S3Error;
use crate::registry::RawStoreRegistry;
use axum::body::Body;
use bytes::Bytes;
use futures_util::StreamExt;
use laoflchdb_raw_store_service::client::RawStoreClientPool;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// 对象元数据中记录的 chunk 引用
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkRef {
    pub id: String,
    pub offset: u64,
    pub size: u64,
    pub sha256: String,
    pub replicas: Vec<String>,
}

pub struct ChunkWriteConfig {
    pub chunk_size: usize,
    pub replication_factor: usize,
    pub write_quorum: usize,
}

/// 流式读取请求体，按 chunk_size 切块并写入 raw_store 副本。
/// 返回 `(chunks, total_size, etag_hex)`。
pub async fn write_object_stream(
    registry: &RawStoreRegistry,
    pool: &RawStoreClientPool,
    cfg: &ChunkWriteConfig,
    body: Body,
) -> Result<(Vec<ChunkRef>, u64, String), S3Error> {
    let chunk_size = cfg.chunk_size.max(1);
    let mut stream = body.into_data_stream();
    let mut buffer: Vec<u8> = Vec::with_capacity(chunk_size);
    let mut chunks: Vec<ChunkRef> = Vec::new();
    let mut offset: u64 = 0;
    let mut total: u64 = 0;
    let mut object_hasher = Sha256::new();

    while let Some(item) = stream.next().await {
        let bytes = item.map_err(|e| S3Error::bad_request(format!("读取请求体失败: {e}")))?;
        buffer.extend_from_slice(&bytes);

        while buffer.len() >= chunk_size {
            let rest = buffer.split_off(chunk_size);
            let data = std::mem::replace(&mut buffer, rest);
            object_hasher.update(&data);
            let chunk = write_chunk(registry, pool, cfg, &data, offset).await?;
            offset += data.len() as u64;
            total += data.len() as u64;
            chunks.push(chunk);
        }
    }

    if !buffer.is_empty() {
        object_hasher.update(&buffer);
        let chunk = write_chunk(registry, pool, cfg, &buffer, offset).await?;
        offset += buffer.len() as u64;
        total += buffer.len() as u64;
        chunks.push(chunk);
    }

    Ok((chunks, total, hex::encode(object_hasher.finalize())))
}

/// 写入单个 chunk 到多个副本
pub async fn write_chunk_data(
    registry: &RawStoreRegistry,
    pool: &RawStoreClientPool,
    cfg: &ChunkWriteConfig,
    data: &[u8],
    offset: u64,
) -> Result<ChunkRef, S3Error> {
    write_chunk(registry, pool, cfg, data, offset).await
}

async fn write_chunk(
    registry: &RawStoreRegistry,
    pool: &RawStoreClientPool,
    cfg: &ChunkWriteConfig,
    data: &[u8],
    offset: u64,
) -> Result<ChunkRef, S3Error> {
    let chunk_id = uuid::Uuid::new_v4().simple().to_string();
    let digest = hex::encode(Sha256::digest(data));

    let replicas = registry
        .replicas_for(&chunk_id, cfg.replication_factor.max(1))
        .await;
    if replicas.is_empty() {
        return Err(S3Error::internal("没有可用的 raw_store 节点"));
    }

    let futures = replicas.iter().map(|addr| {
        let addr = addr.clone();
        let id = chunk_id.clone();
        async move {
            let result = pool.put_chunk(&addr, &id, data).await;
            (addr, result)
        }
    });

    let mut succeeded: Vec<String> = Vec::new();
    for (addr, result) in futures_util::future::join_all(futures).await {
        match result {
            Ok(()) => succeeded.push(addr),
            Err(e) => log::warn!("chunk {chunk_id} 写入 {addr} 失败: {e}"),
        }
    }

    let required = cfg.write_quorum.max(1).min(replicas.len());
    if succeeded.len() < required {
        for addr in &succeeded {
            let _ = pool.delete_chunk(addr, &chunk_id).await;
        }
        return Err(S3Error::internal(format!(
            "chunk {chunk_id} 写入副本数不足: {}/{}",
            succeeded.len(),
            required
        )));
    }

    Ok(ChunkRef {
        id: chunk_id,
        offset,
        size: data.len() as u64,
        sha256: digest,
        replicas: succeeded,
    })
}

/// 从副本中读取并校验一个 chunk
pub async fn fetch_chunk(pool: &RawStoreClientPool, chunk: &ChunkRef) -> Result<Vec<u8>, S3Error> {
    let mut last_error = String::from("无可用副本");
    for addr in &chunk.replicas {
        match pool.get_chunk(addr, &chunk.id).await {
            Ok(Some(data)) => {
                if data.len() as u64 != chunk.size {
                    last_error = format!(
                        "chunk {} 在 {} 长度不匹配: {} != {}",
                        chunk.id,
                        addr,
                        data.len(),
                        chunk.size
                    );
                    continue;
                }
                let actual = hex::encode(Sha256::digest(&data));
                if actual != chunk.sha256 {
                    last_error = format!("chunk {} 在 {} 校验失败", chunk.id, addr);
                    continue;
                }
                return Ok(data);
            }
            Ok(None) => {
                last_error = format!("chunk {} 在 {} 不存在", chunk.id, addr);
            }
            Err(e) => {
                last_error = e;
            }
        }
    }
    Err(S3Error::internal(format!(
        "读取 chunk {} 失败: {}",
        chunk.id, last_error
    )))
}

/// 构造流式读取 Body；`range` 为闭区间 `(start, end)`
pub fn read_object_stream(
    chunks: Vec<ChunkRef>,
    range: Option<(u64, u64)>,
    pool: Arc<RawStoreClientPool>,
) -> Body {
    let stream = async_stream::try_stream! {
        for chunk in chunks {
            let (lo, hi) = match range {
                Some((start, end)) => {
                    let chunk_start = chunk.offset;
                    let chunk_end = chunk.offset + chunk.size;
                    let s = start.max(chunk_start);
                    let e = (end + 1).min(chunk_end);
                    if s >= e {
                        continue;
                    }
                    ((s - chunk_start) as usize, (e - chunk_start) as usize)
                }
                None => (0usize, chunk.size as usize),
            };

            let data = fetch_chunk(&pool, &chunk).await?;
            yield Bytes::from(data[lo..hi].to_vec());
        }
    };

    body_from_stream(stream)
}

/// 用明确的错误类型固定 `try_stream!` 的推断结果，避免 E0283
fn body_from_stream<S>(stream: S) -> Body
where
    S: futures_util::Stream<Item = Result<Bytes, S3Error>> + Send + 'static,
{
    Body::from_stream(stream)
}

/// 尽力删除对象的所有 chunk 副本
pub async fn delete_chunks(pool: &RawStoreClientPool, chunks: &[ChunkRef]) {
    for chunk in chunks {
        for addr in &chunk.replicas {
            if let Err(e) = pool.delete_chunk(addr, &chunk.id).await {
                log::warn!("删除 chunk {}@{} 失败: {e}", chunk.id, addr);
            }
        }
    }
}
