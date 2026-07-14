//! gRPC 客户端封装
//!
//! 统一管理四个服务的 gRPC 客户端（主服务/图片/人脸/向量索引），
//! 共享同一个 Channel（默认端口 19777），并提供统一的认证请求构造方法。

use anyhow::{anyhow, Result};
use tonic::transport::Channel;
use tonic::Request;

use laoflchdb_client::pb::rpc::laoflch_db_client::LaoflchDbClient;
use laoflchdb_client::pb::rpc::LoginRequest;

use laoflchdb_image_service_proto::proto::image_service_client::ImageServiceClient;
use laoflchdb_face_service_proto::proto::face_service_client::FaceServiceClient;
use laoflchdb_embedding_service_proto::proto::embedding_index_service_client::EmbeddingIndexServiceClient;

/// 所有 gRPC 客户端的集合
///
/// 单一 Channel 复用，所有服务（主服务、图片、人脸、向量）共用一个 TCP 连接。
/// token 在登录后保存，后续请求通过 `auth_request` 注入到 metadata 中。
pub struct GrpcClients {
    /// 主服务客户端（认证 / SQL / Schema / 全文索引）
    pub laoflchdb: LaoflchDbClient<Channel>,
    /// 图片服务客户端
    pub image: ImageServiceClient<Channel>,
    /// 人脸服务客户端
    pub face: FaceServiceClient<Channel>,
    /// 向量索引服务客户端
    pub embedding: EmbeddingIndexServiceClient<Channel>,
    /// 登录成功后保存的 token
    pub token: Option<String>,
}

impl GrpcClients {
    /// 连接到指定 host（如 "127.0.0.1:19777"），返回初始化好的 GrpcClients
    pub async fn connect(host: &str) -> Result<Self> {
        let url = format!("http://{}", host);
        let channel = Channel::from_shared(url)?
            .connect()
            .await
            .map_err(|e| anyhow!("连接 {} 失败: {}", host, e))?;

        Ok(Self {
            // 每个客户端 clone Channel（Channel 内部是 Arc，clone 很廉价）
            laoflchdb: LaoflchDbClient::new(channel.clone()),
            image: ImageServiceClient::new(channel.clone()),
            face: FaceServiceClient::new(channel.clone()),
            embedding: EmbeddingIndexServiceClient::new(channel),
            token: None,
        })
    }

    /// 构造一个带认证 token 的 gRPC 请求
    ///
    /// 如果已登录，会在 metadata 中注入 `authorization: Bearer {token}`。
    /// 未登录时返回原始请求（调用方需自行判断是否需要登录）。
    pub fn auth_request<T>(&self, req: T) -> Request<T> {
        let mut request = Request::new(req);
        if let Some(ref token) = self.token {
            let value = format!("Bearer {}", token)
                .parse()
                .expect("token 必须是合法的 header value");
            request.metadata_mut().insert("authorization", value);
        }
        request
    }

    /// 登录并保存 token
    ///
    /// 成功返回 Ok(())，token 会保存到 self.token；
    /// 失败（success=false 或网络错误）返回 Err。
    pub async fn login(&mut self, username: &str, password: &str) -> Result<()> {
        let req = LoginRequest {
            username: username.to_string(),
            password: password.to_string(),
        };
        let resp = self
            .laoflchdb
            .login(req)
            .await
            .map_err(|e| anyhow!("登录请求失败: {}", e))?;
        let resp = resp.into_inner();
        if !resp.success {
            return Err(anyhow!("登录失败: {}", resp.message));
        }
        self.token = Some(resp.token);
        Ok(())
    }

    /// 退出登录（清除本地 token，不调用服务端 Logout 以避免阻塞）
    pub fn logout(&mut self) {
        self.token = None;
    }

    /// 是否已登录
    pub fn is_logged_in(&self) -> bool {
        self.token.is_some()
    }
}
