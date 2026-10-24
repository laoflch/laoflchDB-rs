//! laoflchdb_graph_service_proto
//!
//! 图服务 gRPC 协议定义（GraphQL 执行接口）。
//! 生成的服务端与客户端代码统一放在 [`proto`] 模块下。

pub mod proto {
    tonic::include_proto!("laoflchdb.graph");
}

pub use proto::*;
