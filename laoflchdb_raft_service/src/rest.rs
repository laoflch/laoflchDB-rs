//! Raft 服务 REST 接口
//!
//! 挂载到主 HTTP 服务的 `/api/v1/raft` 下（路由使用根相对路径，由上层 nest）。
//! 接口只以 `group_id` 区分数据组，key 为原始值，不做任何 table 前缀编码。

use crate::RaftKvServiceImpl;
use axum::{
    Router,
    extract::{Query, State},
    response::Json,
    routing::{delete, get, put},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone)]
struct RaftRestState {
    svc: Arc<RaftKvServiceImpl>,
}

#[derive(Serialize)]
pub struct ApiResponse<T> {
    success: bool,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<T>,
}

impl<T> ApiResponse<T> {
    fn ok(data: T) -> Self {
        Self {
            success: true,
            message: String::new(),
            data: Some(data),
        }
    }

    fn err(msg: String) -> Self {
        Self {
            success: false,
            message: msg,
            data: None,
        }
    }
}

/// 创建 Raft REST Router（根相对路径，nest 到 "/api/v1/raft" 下）
pub fn create_rest_router(svc: Arc<RaftKvServiceImpl>) -> Router {
    let state = RaftRestState { svc };
    Router::new()
        .route("/kv", put(put_kv))
        .route("/kv", get(get_kv))
        .route("/kv", delete(delete_kv))
        .route("/kv/scan", get(scan_kv))
        .route("/status", get(status))
        .with_state(state)
}

// ── KV ──

#[derive(Deserialize)]
pub struct PutKvBody {
    pub group_id: String,
    pub key: String,
    pub value: String,
}

async fn put_kv(
    State(state): State<RaftRestState>,
    Json(body): Json<PutKvBody>,
) -> Json<ApiResponse<bool>> {
    match state
        .svc
        .kv_put(&body.group_id, body.key.into_bytes(), body.value.into_bytes())
        .await
    {
        Ok(success) => Json(ApiResponse::ok(success)),
        Err(e) => Json(ApiResponse::err(e)),
    }
}

#[derive(Deserialize)]
pub struct KvQuery {
    pub group_id: String,
    pub key: String,
}

async fn get_kv(
    State(state): State<RaftRestState>,
    Query(q): Query<KvQuery>,
) -> Json<ApiResponse<Option<String>>> {
    match state.svc.kv_get(&q.group_id, q.key.as_bytes()).await {
        Ok(Some(v)) => Json(ApiResponse::ok(Some(String::from_utf8_lossy(&v).into_owned()))),
        Ok(None) => Json(ApiResponse::ok(None)),
        Err(e) => Json(ApiResponse::err(e)),
    }
}

async fn delete_kv(
    State(state): State<RaftRestState>,
    Query(q): Query<KvQuery>,
) -> Json<ApiResponse<bool>> {
    match state.svc.kv_delete(&q.group_id, q.key.into_bytes()).await {
        Ok(success) => Json(ApiResponse::ok(success)),
        Err(e) => Json(ApiResponse::err(e)),
    }
}

#[derive(Deserialize)]
pub struct ScanQuery {
    pub group_id: String,
}

#[derive(Serialize)]
pub struct KvEntry {
    pub key: String,
    pub value: String,
}

async fn scan_kv(
    State(state): State<RaftRestState>,
    Query(q): Query<ScanQuery>,
) -> Json<ApiResponse<Vec<KvEntry>>> {
    match state.svc.kv_scan(&q.group_id).await {
        Ok(entries) => {
            let list: Vec<KvEntry> = entries
                .into_iter()
                .map(|(k, v)| KvEntry {
                    key: String::from_utf8_lossy(&k).into_owned(),
                    value: String::from_utf8_lossy(&v).into_owned(),
                })
                .collect();
            Json(ApiResponse::ok(list))
        }
        Err(e) => Json(ApiResponse::err(e)),
    }
}

// ── 状态 ──

#[derive(Deserialize)]
pub struct StatusQuery {
    pub group_id: Option<String>,
}

#[derive(Serialize)]
pub struct GroupStatus {
    pub group_id: String,
    pub status: String,
}

/// 组状态：不传 group_id 时返回本节点全部组
async fn status(
    State(state): State<RaftRestState>,
    Query(q): Query<StatusQuery>,
) -> Json<ApiResponse<Vec<GroupStatus>>> {
    let ids: Vec<String> = match q.group_id {
        Some(g) => vec![g],
        None => state.svc.group_ids(),
    };
    let mut groups = Vec::new();
    for g in ids {
        match state.svc.node_status(&g).await {
            Ok(s) => groups.push(GroupStatus { group_id: g, status: s }),
            Err(e) => groups.push(GroupStatus {
                group_id: g,
                status: format!("错误: {}", e),
            }),
        }
    }
    Json(ApiResponse::ok(groups))
}
