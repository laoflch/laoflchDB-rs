//! 轻量级 laoflchdb gRPC 客户端 proto 代码
//!
//! 仅包含 proto 生成的 client 端代码，不依赖 rocksdb 等重型依赖。
//! 供 ltool、lsql 等客户端工具复用。

/// Proto 生成的 gRPC 类型和客户端
pub mod pb {
    pub mod rpc {
        tonic::include_proto!("laoflchdb.rpc");
    }
}