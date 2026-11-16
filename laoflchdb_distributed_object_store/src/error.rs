//! 分布式对象存储统一错误类型

use axum::http::StatusCode;
use std::fmt;

#[derive(Debug, Clone)]
pub struct S3Error {
    pub status: StatusCode,
    pub code: String,
    pub message: String,
    /// 416 响应需要返回的实际对象大小
    pub actual_size: Option<u64>,
}

impl S3Error {
    pub fn new(status: StatusCode, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into(),
            actual_size: None,
        }
    }

    pub fn no_such_bucket(bucket: &str) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "NoSuchBucket",
            format!("The specified bucket does not exist: {bucket}"),
        )
    }

    pub fn no_such_key(bucket: &str, key: &str) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "NoSuchKey",
            format!("The specified key does not exist: {bucket}/{key}"),
        )
    }

    pub fn no_such_policy() -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "NoSuchBucketPolicy",
            "The bucket policy does not exist",
        )
    }

    pub fn bucket_not_empty(bucket: &str) -> Self {
        Self::new(
            StatusCode::CONFLICT,
            "BucketNotEmpty",
            format!("The bucket you tried to delete is not empty: {bucket}"),
        )
    }

    pub fn access_denied() -> Self {
        Self::new(StatusCode::FORBIDDEN, "AccessDenied", "Access Denied")
    }

    pub fn signature_mismatch() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "SignatureDoesNotMatch",
            "The request signature we calculated does not match the signature you provided",
        )
    }

    pub fn invalid_access_key() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "InvalidAccessKeyId",
            "The AWS Access Key Id you provided does not exist in our records",
        )
    }

    pub fn invalid_range(total: u64) -> Self {
        let mut err = Self::new(
            StatusCode::RANGE_NOT_SATISFIABLE,
            "InvalidRange",
            "The requested range is not satisfiable",
        );
        err.actual_size = Some(total);
        err
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "InvalidRequest", message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "InternalError", message)
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_IMPLEMENTED, "NotImplemented", message)
    }
}

impl fmt::Display for S3Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for S3Error {}
