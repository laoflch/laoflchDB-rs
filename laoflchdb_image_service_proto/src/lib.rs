//! 轻量级 ImageService proto 客户端代码
//!
//! 仅包含 proto 生成的 client 端代码，不依赖对象存储等后端服务。

pub mod proto {
    tonic::include_proto!("laoflchdb.image_service");
}