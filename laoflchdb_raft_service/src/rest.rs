//! Raft 服务 REST 接口
//!
//! 挂载到主 HTTP 服务的 `/api/v1/raft` 下（路由使用根相对路径，由上层 nest）。
//! table 语义与 raft_admin.py 一致：落盘键 = table + NUL + key（table 为空时键为 `\x00` + key）。

use crate::RaftKvServiceImpl;
use axum::{
    Router,
    extract::{Query, State},
    response::Json,
    routing::{delete, get, put},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// 落盘键编码：table + NUL + key（table 为空时仅 `\x00` 前缀），与 raft_admin.py 的 `_data_key` 一致
pub(crate) fn data_key(table: &str, key: &str) -> Vec<u8> {
    let mut k = Vec::with_capacity(table.len() + 1 + key.len());
    k.extend_from_slice(table.as_bytes());
    k.push(0);
    k.extend_from_slice(key.as_bytes());
    k
}

/// 解析落盘键：按第一个 NUL 拆分为 (table, key)；无 NUL 时 table 为空
pub fn split_data_key(dk: &[u8]) -> (String, String) {
    match dk.iter().position(|&b| b == 0) {
        Some(pos) => (
            String::from_utf8_lossy(&dk[..pos]).into_owned(),
            String::from_utf8_lossy(&dk[pos + 1..]).into_owned(),
        ),
        None => (String::new(), String::from_utf8_lossy(dk).into_owned()),
    }
}

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
    pub group_id: u32,
    #[serde(default)]
    pub table: String,
    pub key: String,
    pub value: String,
}

async fn put_kv(
    State(state): State<RaftRestState>,
    Json(body): Json<PutKvBody>,
) -> Json<ApiResponse<bool>> {
    match state
        .svc
        .kv_put(body.group_id, data_key(&body.table, &body.key), body.value.into_bytes())
        .await
    {
        Ok(success) => Json(ApiResponse::ok(success)),
        Err(e) => Json(ApiResponse::err(e)),
    }
}

#[derive(Deserialize)]
pub struct KvQuery {
    pub group_id: u32,
    #[serde(default)]
    pub table: String,
    pub key: String,
}

async fn get_kv(
    State(state): State<RaftRestState>,
    Query(q): Query<KvQuery>,
) -> Json<ApiResponse<Option<String>>> {
    match state.svc.kv_get(q.group_id, &data_key(&q.table, &q.key)).await {
        Ok(Some(v)) => Json(ApiResponse::ok(Some(String::from_utf8_lossy(&v).into_owned()))),
        Ok(None) => Json(ApiResponse::ok(None)),
        Err(e) => Json(ApiResponse::err(e)),
    }
}

async fn delete_kv(
    State(state): State<RaftRestState>,
    Query(q): Query<KvQuery>,
) -> Json<ApiResponse<bool>> {
    match state.svc.kv_delete(q.group_id, data_key(&q.table, &q.key)).await {
        Ok(success) => Json(ApiResponse::ok(success)),
        Err(e) => Json(ApiResponse::err(e)),
    }
}

#[derive(Deserialize)]
pub struct ScanQuery {
    pub group_id: u32,
    #[serde(default)]
    pub table: String,
}

#[derive(Serialize)]
pub struct KvEntry {
    pub table: String,
    pub key: String,
    pub value: String,
}

async fn scan_kv(
    State(state): State<RaftRestState>,
    Query(q): Query<ScanQuery>,
) -> Json<ApiResponse<Vec<KvEntry>>> {
    // 与 raft_admin.py 的 scan 一致：以 "table + NUL" 为前缀过滤（避免 "t1" 误配 "t10"）
    let prefix = if q.table.is_empty() {
        None
    } else {
        Some(data_key(&q.table, ""))
    };
    match state.svc.kv_scan(q.group_id).await {
        Ok(entries) => {
            let list: Vec<KvEntry> = entries
                .into_iter()
                .filter(|(dk, _)| match &prefix {
                    Some(p) => dk.starts_with(p),
                    None => true,
                })
                .map(|(dk, v)| {
                    let (table, key) = split_data_key(&dk);
                    KvEntry {
                        table,
                        key,
                        value: String::from_utf8_lossy(&v).into_owned(),
                    }
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
    pub group_id: Option<u32>,
}

#[derive(Serialize)]
pub struct GroupStatus {
    pub group_id: u32,
    pub status: String,
}

/// 组状态：不传 group_id 时返回本节点全部组
async fn status(
    State(state): State<RaftRestState>,
    Query(q): Query<StatusQuery>,
) -> Json<ApiResponse<Vec<GroupStatus>>> {
    let ids: Vec<u32> = match q.group_id {
        Some(g) => vec![g],
        None => state.svc.group_ids(),
    };
    let mut groups = Vec::new();
    for g in ids {
        match state.svc.node_status(g).await {
            Ok(s) => groups.push(GroupStatus { group_id: g, status: s }),
            Err(e) => groups.push(GroupStatus {
                group_id: g,
                status: format!("错误: {}", e),
            }),
        }
    }
    Json(ApiResponse::ok(groups))
}
