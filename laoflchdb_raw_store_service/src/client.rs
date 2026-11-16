//! raw_store gRPC 客户端连接池

use crate::proto::raw_store_service_client::RawStoreServiceClient;
use crate::proto::{
    DeleteChunkRequest, GetChunkRequest, HealthRequest, PutChunkRequest, StatChunkRequest,
};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::Mutex;

const MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024;

/// 连接池：按地址缓存 tonic channel，RPC 失败后自动失效并在下次调用时重连。
pub struct RawStoreClientPool {
    clients: Mutex<HashMap<String, RawStoreServiceClient<tonic::transport::Channel>>>,
}

impl Default for RawStoreClientPool {
    fn default() -> Self {
        Self::new()
    }
}

impl RawStoreClientPool {
    pub fn new() -> Self {
        Self {
            clients: Mutex::new(HashMap::new()),
        }
    }

    fn endpoint_url(addr: &str) -> String {
        if addr.starts_with("http://") || addr.starts_with("https://") {
            addr.to_string()
        } else {
            format!("http://{addr}")
        }
    }

    async fn client(
        &self,
        addr: &str,
    ) -> Result<RawStoreServiceClient<tonic::transport::Channel>, String> {
        {
            let clients = self.clients.lock().await;
            if let Some(client) = clients.get(addr) {
                return Ok(client.clone());
            }
        }

        let endpoint = tonic::transport::Endpoint::from_shared(Self::endpoint_url(addr))
            .map_err(|e| format!("raw_store 地址非法 {addr}: {e}"))?
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(30))
            .initial_stream_window_size(MAX_MESSAGE_SIZE as u32)
            .initial_connection_window_size(MAX_MESSAGE_SIZE as u32);

        let channel = endpoint
            .connect()
            .await
            .map_err(|e| format!("连接 raw_store {addr} 失败: {e}"))?;

        let client = RawStoreServiceClient::new(channel)
            .max_decoding_message_size(MAX_MESSAGE_SIZE)
            .max_encoding_message_size(MAX_MESSAGE_SIZE);

        self.clients
            .lock()
            .await
            .insert(addr.to_string(), client.clone());
        Ok(client)
    }

    async fn invalidate(&self, addr: &str) {
        self.clients.lock().await.remove(addr);
    }

    pub async fn put_chunk(&self, addr: &str, chunk_id: &str, data: &[u8]) -> Result<(), String> {
        let mut client = self.client(addr).await?;
        let request = tonic::Request::new(PutChunkRequest {
            chunk_id: chunk_id.to_string(),
            data: data.to_vec(),
        });
        match client.put_chunk(request).await {
            Ok(resp) => {
                let resp = resp.into_inner();
                if resp.success {
                    Ok(())
                } else {
                    Err(format!("raw_store {addr} 写入 chunk 失败: {}", resp.message))
                }
            }
            Err(e) => {
                self.invalidate(addr).await;
                Err(format!("raw_store {addr} PutChunk RPC 失败: {e}"))
            }
        }
    }

    pub async fn get_chunk(&self, addr: &str, chunk_id: &str) -> Result<Option<Vec<u8>>, String> {
        let mut client = self.client(addr).await?;
        let request = tonic::Request::new(GetChunkRequest {
            chunk_id: chunk_id.to_string(),
        });
        match client.get_chunk(request).await {
            Ok(resp) => {
                let resp = resp.into_inner();
                if resp.found {
                    Ok(Some(resp.data))
                } else {
                    Ok(None)
                }
            }
            Err(e) => {
                self.invalidate(addr).await;
                Err(format!("raw_store {addr} GetChunk RPC 失败: {e}"))
            }
        }
    }

    pub async fn delete_chunk(&self, addr: &str, chunk_id: &str) -> Result<(), String> {
        let mut client = self.client(addr).await?;
        let request = tonic::Request::new(DeleteChunkRequest {
            chunk_id: chunk_id.to_string(),
        });
        match client.delete_chunk(request).await {
            Ok(_) => Ok(()),
            Err(e) => {
                self.invalidate(addr).await;
                Err(format!("raw_store {addr} DeleteChunk RPC 失败: {e}"))
            }
        }
    }

    pub async fn stat_chunk(
        &self,
        addr: &str,
        chunk_id: &str,
    ) -> Result<Option<(u64, Vec<u8>)>, String> {
        let mut client = self.client(addr).await?;
        let request = tonic::Request::new(StatChunkRequest {
            chunk_id: chunk_id.to_string(),
        });
        match client.stat_chunk(request).await {
            Ok(resp) => {
                let resp = resp.into_inner();
                if resp.found {
                    Ok(Some((resp.size, resp.sha256)))
                } else {
                    Ok(None)
                }
            }
            Err(e) => {
                self.invalidate(addr).await;
                Err(format!("raw_store {addr} StatChunk RPC 失败: {e}"))
            }
        }
    }

    pub async fn health(&self, addr: &str) -> Result<u64, String> {
        let mut client = self.client(addr).await?;
        match client.health(tonic::Request::new(HealthRequest {})).await {
            Ok(resp) => {
                let resp = resp.into_inner();
                if resp.healthy {
                    Ok(resp.node_id)
                } else {
                    Err(format!("raw_store {addr} 不健康"))
                }
            }
            Err(e) => {
                self.invalidate(addr).await;
                Err(format!("raw_store {addr} Health RPC 失败: {e}"))
            }
        }
    }
}
