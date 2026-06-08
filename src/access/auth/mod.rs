use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::time::{SystemTime, Duration};

/// Token 信息
#[derive(Clone)]
pub struct TokenInfo {
    pub user_id: i64,
    pub username: String,
    pub created_at: SystemTime,
    pub expires_at: SystemTime,
}

impl TokenInfo {
    pub fn new(user_id: i64, username: String, ttl_hours: u64) -> Self {
        let now = SystemTime::now();
        Self {
            user_id,
            username,
            created_at: now,
            expires_at: now + Duration::from_secs(ttl_hours * 3600),
        }
    }

    pub fn is_expired(&self) -> bool {
        SystemTime::now() > self.expires_at
    }
}

/// Token 管理器
pub struct TokenManager {
    tokens: Arc<RwLock<HashMap<String, TokenInfo>>>,
    default_ttl_hours: u64,
}

impl TokenManager {
    pub fn new(ttl_hours: u64) -> Self {
        Self {
            tokens: Arc::new(RwLock::new(HashMap::new())),
            default_ttl_hours: ttl_hours,
        }
    }

    /// 生成新 token
    pub async fn generate_token(&self, user_id: i64, username: String) -> String {
        let token = uuid::Uuid::new_v4().to_string();
        let token_info = TokenInfo::new(user_id, username, self.default_ttl_hours);
        
        let mut tokens = self.tokens.write().await;
        tokens.insert(token.clone(), token_info);
        
        token
    }

    /// 验证 token
    pub async fn validate_token(&self, token: &str) -> Option<TokenInfo> {
        let tokens = self.tokens.read().await;
        
        if let Some(token_info) = tokens.get(token) {
            if !token_info.is_expired() {
                return Some(token_info.clone());
            }
        }
        
        None
    }

    /// 注销 token
    pub async fn revoke_token(&self, token: &str) -> bool {
        let mut tokens = self.tokens.write().await;
        tokens.remove(token).is_some()
    }

    /// 清理过期 token
    pub async fn cleanup_expired(&self) {
        let mut tokens = self.tokens.write().await;
        tokens.retain(|_, info| !info.is_expired());
    }

    /// 获取活跃 token 数量
    pub async fn active_tokens_count(&self) -> usize {
        let tokens = self.tokens.read().await;
        tokens.len()
    }
}

impl Default for TokenManager {
    fn default() -> Self {
        Self::new(24) // 默认 24 小时过期
    }
}
