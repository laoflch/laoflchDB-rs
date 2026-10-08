//! Raft 类型配置
//!
//! 定义 Request/Response/NodeId 等泛型参数，供 openraft 0.9 使用。

use serde::{Deserialize, Serialize};

// ── 节点 ID（直接使用 u64，无需实现 trait）──
pub type NodeId = u64;

// ── Raft 请求（写操作）──
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Request {
    Put {
        table: String,
        key: Vec<u8>,
        value: Vec<u8>,
    },
    Delete {
        table: String,
        key: Vec<u8>,
    },
}

// 注：openraft 对 AppData / AppDataResponse 提供了 blanket impl
// （T: OptionalSend + OptionalSync + 'static + OptionalSerde），Request/Response
// 已满足这些约束，无需（也不能）再手写 impl。

// ── Raft 响应 ──
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub success: bool,
    pub message: String,
}

// ── 类型配置 ──
openraft::declare_raft_types!(
    pub TypeConfig:
        D = Request,
        R = Response,
        NodeId = NodeId,
        Node = openraft::BasicNode,
        Entry = openraft::Entry<TypeConfig>,
        SnapshotData = Vec<u8>,
        AsyncRuntime = openraft::TokioRuntime,
);

pub type Raft = openraft::Raft<TypeConfig>;
