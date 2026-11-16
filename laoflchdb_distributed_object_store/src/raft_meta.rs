//! Raft 元数据客户端（复用现有 Raft KV gRPC 接口，不修改 Raft 服务）

use crate::error::S3Error;
use laoflchdb_raft_service::proto::raft_kv_service_client::RaftKvServiceClient;
use laoflchdb_raft_service::proto::{DeleteRequest, GetRequest, PutRequest, ScanRequest};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

const MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024;

#[derive(Clone)]
pub struct RaftMetaClient {
    endpoints: Arc<Vec<String>>,
    group_id: Arc<String>,
    clients: Arc<Mutex<HashMap<String, RaftKvServiceClient<tonic::transport::Channel>>>>,
}

impl RaftMetaClient {
    pub fn new(endpoints: Vec<String>, group_id: String) -> Self {
        Self {
            endpoints: Arc::new(endpoints),
            group_id: Arc::new(group_id),
            clients: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn group_id(&self) -> &str {
        &self.group_id
    }

    fn endpoint_url(endpoint: &str) -> String {
        if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
            endpoint.to_string()
        } else {
            format!("http://{endpoint}")
        }
    }

    async fn client(
        &self,
        endpoint: &str,
    ) -> Result<RaftKvServiceClient<tonic::transport::Channel>, String> {
        {
            let clients = self.clients.lock().await;
            if let Some(client) = clients.get(endpoint) {
                return Ok(client.clone());
            }
        }

        let channel = tonic::transport::Endpoint::from_shared(Self::endpoint_url(endpoint))
            .map_err(|e| format!("Raft endpoint 地址非法 {endpoint}: {e}"))?
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(15))
            .initial_stream_window_size(MAX_MESSAGE_SIZE as u32)
            .initial_connection_window_size(MAX_MESSAGE_SIZE as u32)
            .connect()
            .await
            .map_err(|e| format!("连接 Raft endpoint {endpoint} 失败: {e}"))?;

        let client = RaftKvServiceClient::new(channel)
            .max_decoding_message_size(MAX_MESSAGE_SIZE)
            .max_encoding_message_size(MAX_MESSAGE_SIZE);

        self.clients
            .lock()
            .await
            .insert(endpoint.to_string(), client.clone());
        Ok(client)
    }

    async fn invalidate(&self, endpoint: &str) {
        self.clients.lock().await.remove(endpoint);
    }

    pub async fn put(&self, key: Vec<u8>, value: Vec<u8>) -> Result<(), S3Error> {
        let mut last_error = String::from("没有可用的 Raft endpoint");
        for endpoint in self.endpoints.iter() {
            let mut client = match self.client(endpoint).await {
                Ok(client) => client,
                Err(e) => {
                    last_error = e;
                    continue;
                }
            };

            let request = PutRequest {
                key: key.clone(),
                value: value.clone(),
                group_id: self.group_id.to_string(),
            };

            match client.put(request).await {
                Ok(resp) => {
                    let resp = resp.into_inner();
                    if resp.success {
                        return Ok(());
                    }
                    last_error = format!("Raft Put 失败: {}", resp.message);
                }
                Err(e) => {
                    self.invalidate(endpoint).await;
                    last_error = format!("Raft Put RPC 失败: {e}");
                }
            }
        }
        Err(S3Error::internal(last_error))
    }

    pub async fn get(&self, key: Vec<u8>) -> Result<Option<Vec<u8>>, S3Error> {
        let mut last_error = String::from("没有可用的 Raft endpoint");
        let mut any_success = false;

        for endpoint in self.endpoints.iter() {
            let mut client = match self.client(endpoint).await {
                Ok(client) => client,
                Err(e) => {
                    last_error = e;
                    continue;
                }
            };

            let request = GetRequest {
                key: key.clone(),
                group_id: self.group_id.to_string(),
            };

            match client.get(request).await {
                Ok(resp) => {
                    any_success = true;
                    let resp = resp.into_inner();
                    if resp.found {
                        return Ok(Some(resp.value));
                    }
                }
                Err(e) => {
                    self.invalidate(endpoint).await;
                    last_error = format!("Raft Get RPC 失败: {e}");
                }
            }
        }

        if any_success {
            Ok(None)
        } else {
            Err(S3Error::internal(last_error))
        }
    }

    pub async fn delete(&self, key: Vec<u8>) -> Result<(), S3Error> {
        let mut last_error = String::from("没有可用的 Raft endpoint");
        for endpoint in self.endpoints.iter() {
            let mut client = match self.client(endpoint).await {
                Ok(client) => client,
                Err(e) => {
                    last_error = e;
                    continue;
                }
            };

            let request = DeleteRequest {
                key: key.clone(),
                group_id: self.group_id.to_string(),
            };

            match client.delete(request).await {
                Ok(resp) => {
                    let resp = resp.into_inner();
                    if resp.success {
                        return Ok(());
                    }
                    last_error = "Raft Delete 返回失败".to_string();
                }
                Err(e) => {
                    self.invalidate(endpoint).await;
                    last_error = format!("Raft Delete RPC 失败: {e}");
                }
            }
        }
        Err(S3Error::internal(last_error))
    }

    pub async fn scan(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>, S3Error> {
        let mut last_error = String::from("没有可用的 Raft endpoint");
        for endpoint in self.endpoints.iter() {
            let mut client = match self.client(endpoint).await {
                Ok(client) => client,
                Err(e) => {
                    last_error = e;
                    continue;
                }
            };

            let request = ScanRequest {
                group_id: self.group_id.to_string(),
            };

            match client.scan(request).await {
                Ok(resp) => {
                    let entries = resp
                        .into_inner()
                        .entries
                        .into_iter()
                        .map(|e| (e.key, e.value))
                        .collect();
                    return Ok(entries);
                }
                Err(e) => {
                    self.invalidate(endpoint).await;
                    last_error = format!("Raft Scan RPC 失败: {e}");
                }
            }
        }
        Err(S3Error::internal(last_error))
    }
}
