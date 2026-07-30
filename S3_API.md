# LaoflchDB S3 兼容 API 文档

## 概述

LaoflchDB 提供完全兼容 S3 协议的对象存储接口，支持以下特性：

- **AWS Signature V4** 签名验证
- **标准 S3 XML** 响应格式
- **独立 S3 路由**（与主 API 分开）
- **URL 编码键**（支持包含空格、中文等特殊字符）
- **预签名 URL** 支持

## 基础信息

| 项目 | 值 |
|------|-----|
| **Base URL** | `http://localhost:9000` |
| **默认 Access Key** | `admin` |
| **默认 Secret Key** | `laoflchdb` |
| **默认 Region** | `us-east-1` |
| **协议** | `HTTP/1.1` |
| **内容格式** | `XML` |

## 配置

在 `laoflchdb.yaml` 中配置 S3 服务：

```yaml
# S3 服务配置
access_protocols:
  - protocol: s3
    enabled: true
    addr: 0.0.0.0:9000
    service_id: s3_admin
    s3_access_key: admin       # 自定义 Access Key
    s3_secret_key: laoflchdb   # 自定义 Secret Key
```

## 认证方式

### 1. Authorization 头认证

```bash
# 使用 AWS Signature V4 签名
curl "http://localhost:9000/my-bucket" \
  -H "Authorization: AWS4-HMAC-SHA256 Credential=admin/20260730/us-east-1/s3/aws4_request, SignedHeaders=host, Signature=..." \
  -H "x-amz-date: 20260730T000000Z"
```

### 2. 预签名 URL 认证

```bash
# 预签名 URL（无需额外认证头）
curl "http://localhost:9000/my-bucket/object.txt?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=admin%2F20260730%2Fus-east-1%2Fs3%2Faws4_request&X-Amz-Date=20260730T000000Z&X-Amz-Expires=3600&X-Amz-SignedHeaders=host&X-Amz-Signature=..."
```

## API 端点

### 1. ListBuckets - 列出所有 Bucket

**端点**: `GET /`

**响应示例**:
```xml
<?xml version="1.0" encoding="UTF-8"?>
<ListAllMyBucketsResult>
  <Buckets>
    <Bucket>
      <Name>images</Name>
      <CreationDate>2026-07-30T00:00:00Z</CreationDate>
    </Bucket>
    <Bucket>
      <Name>documents</Name>
      <CreationDate>2026-07-30T00:00:00Z</CreationDate>
    </Bucket>
  </Buckets>
</ListAllMyBucketsResult>
```

---

### 2. CreateBucket - 创建 Bucket

**端点**: `PUT /{bucket}`

**响应**: 成功返回 `200 OK`，空响应体。

**说明**: 重复创建同一 Bucket 是幂等操作。

---

### 3. DeleteBucket - 删除 Bucket

**端点**: `DELETE /{bucket}`

**响应**: 成功返回 `204 No Content`。

---

### 4. HeadBucket - 获取 Bucket 元数据

**端点**: `HEAD /{bucket}`

**响应**: 成功返回 `200 OK`，空响应体。
- 不存在的 Bucket 返回 `404 Not Found`

---

### 5. ListObjects - 列出 Bucket 中的对象

**端点**: `GET /{bucket}`

**查询参数**:

| 参数 | 类型 | 说明 |
|------|------|------|
| `prefix` | string | 仅列出键以该前缀开头的对象 |
| `delimiter` | string | 目录分隔符（通常为 `/`），用于模拟目录结构 |
| `max-keys` | int32 | 返回对象数量上限（默认 1000） |
| `marker` | string | 分页起始键（从上一次 `NextMarker` 继续） |
| `continuation-token` | string | ListObjects V2 分页起始键 |
| `list-type` | int | 设为 2 启用 ListObjects V2 |
| `encoding-type` | string | 键编码方式（通常为 `url`） |

**响应示例** (ListObjects V2):
```xml
<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult>
  <Name>my-bucket</Name>
  <Prefix/>
  <MaxKeys>100</MaxKeys>
  <IsTruncated>false</IsTruncated>
  <Contents>
    <Key>photos/cat.jpg</Key>
    <LastModified>2026-07-30T00:00:00Z</LastModified>
    <ETag>&quot;a1b2c3d4e5f6&quot;</ETag>
    <Size>102400</Size>
    <StorageClass>STANDARD</StorageClass>
  </Contents>
  <CommonPrefixes>
    <Prefix>photos/2024/</Prefix>
  </CommonPrefixes>
</ListBucketResult>
```

---

### 6. PutObject - 上传对象

**端点**: `PUT /{bucket}/{key}`

**请求头**:
- `Content-Type`: 对象的 MIME 类型（默认 `application/octet-stream`）
- `Content-Length`: 对象字节数

**请求体**: 原始二进制数据（任意字节）

**响应示例**:
```xml
<?xml version="1.0" encoding="UTF-8"?>
<PutObjectResult>
  <ETag>&quot;a1b2c3d4e5f6&quot;</ETag>
</PutObjectResult>
```

**说明**: 
- 每次上传都会生成新的 ETag（基于 UUID）
- 覆盖已有对象时 ETag 会更新
- 支持大对象上传（基于 RocksDB BlobDB）
- 支持 URL 编码键（如 `photos/my photo.jpg`）

---

### 7. GetObject - 下载对象

**端点**: `GET /{bucket}/{key}`

**响应头**:
- `Content-Type`: 对象的 MIME 类型
- `Content-Length`: 对象字节数
- `ETag`: 对象的 ETag
- `Last-Modified`: 最后修改时间

**响应体**: 原始二进制数据

**错误**: 对象不存在时返回 `404 Not Found`。

---

### 8. HeadObject - 获取对象元数据

**端点**: `HEAD /{bucket}/{key}`

**响应头**:
- `Content-Type`: 对象的 MIME 类型
- `Content-Length`: 对象字节数
- `ETag`: 对象的 ETag
- `Last-Modified`: 最后修改时间

**响应体**: 空

**错误**: 对象不存在时返回 `404 Not Found`。

---

### 9. DeleteObject - 删除对象

**端点**: `DELETE /{bucket}/{key}`

**响应**: 成功返回 `204 No Content`。

**说明**: 删除不存在的对象是幂等操作，始终返回成功。

---

## Python 示例 (boto3)

```python
import boto3

# 创建 S3 客户端
s3 = boto3.client(
    's3',
    endpoint_url='http://localhost:9000',
    aws_access_key_id='admin',
    aws_secret_access_key='laoflchdb',
    region_name='us-east-1'
)

# 1. 列出所有 Bucket
buckets = s3.list_buckets()
for bucket in buckets['Buckets']:
    print(f"Bucket: {bucket['Name']}")

# 2. 创建 Bucket
s3.create_bucket(Bucket='my-bucket')

# 3. 上传文件
with open('/path/to/file.txt', 'rb') as f:
    s3.put_object(Bucket='my-bucket', Key='file.txt', Body=f)

# 4. 下载文件
obj = s3.get_object(Bucket='my-bucket', Key='file.txt')
with open('/path/to/downloaded.txt', 'wb') as f:
    f.write(obj['Body'].read())

# 5. 列出对象
objects = s3.list_objects_v2(Bucket='my-bucket', Prefix='photos/')
for obj in objects.get('Contents', []):
    print(f"Object: {obj['Key']}, Size: {obj['Size']}")

# 6. 删除对象
s3.delete_object(Bucket='my-bucket', Key='file.txt')

# 7. 删除 Bucket
s3.delete_bucket(Bucket='my-bucket')
```

---

## Rust 示例 (rusty-s3)

```rust
use rusty_s3::{Bucket, Credentials, UrlStyle};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 创建 Bucket 配置
    let bucket = Bucket::new(
        Url::parse("http://localhost:9000/my-bucket")?,
        UrlStyle::Path,
        "us-east-1".to_string(),
    )?;
    
    let credentials = Credentials::new("admin", "laoflchdb");

    // 1. 列出对象
    let action = bucket.list_objects_v2(Some(&credentials));
    let url = action.sign(Duration::from_secs(3600));
    let resp = reqwest::get(url).await?;
    let xml = resp.text().await?;
    println!("Objects: {}", xml);

    // 2. 上传文件
    let action = bucket.put_object(Some(&credentials), "test.txt");
    let url = action.sign(Duration::from_secs(3600));
    let client = reqwest::Client::new();
    let resp = client
        .put(url)
        .body("Hello, S3!")
        .header("Content-Type", "text/plain")
        .send()
        .await?;
    println!("Upload status: {}", resp.status());

    // 3. 下载文件
    let action = bucket.get_object(Some(&credentials), "test.txt");
    let url = action.sign(Duration::from_secs(3600));
    let resp = reqwest::get(url).await?;
    let content = resp.text().await?;
    println!("Content: {}", content);

    Ok(())
}
```

---

## cURL 示例

```bash
# 1. 列出所有 Bucket（使用预签名 URL）
curl "http://localhost:9000?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=admin%2F20260730%2Fus-east-1%2Fs3%2Faws4_request&X-Amz-Date=20260730T000000Z&X-Amz-Expires=3600&X-Amz-SignedHeaders=host&X-Amz-Signature=..."

# 2. 创建 Bucket
curl -X PUT "http://localhost:9000/my-bucket?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=admin%2F20260730%2Fus-east-1%2Fs3%2Faws4_request&X-Amz-Date=20260730T000000Z&X-Amz-Expires=3600&X-Amz-SignedHeaders=host&X-Amz-Signature=..."

# 3. 上传文件
curl -X PUT "http://localhost:9000/my-bucket/photos/cat.jpg?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=admin%2F20260730%2Fus-east-1%2Fs3%2Faws4_request&X-Amz-Date=20260730T000000Z&X-Amz-Expires=3600&X-Amz-SignedHeaders=host&X-Amz-Signature=..." \
  -H "Content-Type: image/jpeg" \
  --data-binary @/path/to/cat.jpg

# 4. 下载文件
curl "http://localhost:9000/my-bucket/photos/cat.jpg?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=admin%2F20260730%2Fus-east-1%2Fs3%2Faws4_request&X-Amz-Date=20260730T000000Z&X-Amz-Expires=3600&X-Amz-SignedHeaders=host&X-Amz-Signature=..." \
  -o cat.jpg

# 5. 获取对象元数据
curl -I "http://localhost:9000/my-bucket/photos/cat.jpg?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=admin%2F20260730%2Fus-east-1%2Fs3%2Faws4_request&X-Amz-Date=20260730T000000Z&X-Amz-Expires=3600&X-Amz-SignedHeaders=host&X-Amz-Signature=..."

# 6. 列出 Bucket 中的对象
curl "http://localhost:9000/my-bucket?prefix=photos/&delimiter=/&X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=admin%2F20260730%2Fus-east-1%2Fs3%2Faws4_request&X-Amz-Date=20260730T000000Z&X-Amz-Expires=3600&X-Amz-SignedHeaders=host&X-Amz-Signature=..."

# 7. 删除对象
curl -X DELETE "http://localhost:9000/my-bucket/photos/cat.jpg?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=admin%2F20260730%2Fus-east-1%2Fs3%2Faws4_request&X-Amz-Date=20260730T000000Z&X-Amz-Expires=3600&X-Amz-SignedHeaders=host&X-Amz-Signature=..."

# 8. 删除 Bucket
curl -X DELETE "http://localhost:9000/my-bucket?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=admin%2F20260730%2Fus-east-1%2Fs3%2Faws4_request&X-Amz-Date=20260730T000000Z&X-Amz-Expires=3600&X-Amz-SignedHeaders=host&X-Amz-Signature=..."
```

---

## 特殊字符处理

### URL 编码键

S3 协议要求键必须 URL 编码，支持以下场景：

| 原始键 | URL 编码 |
|--------|----------|
| `my photo.jpg` | `my%20photo.jpg` |
| `文档.pdf` | `%E6%96%87%E6%A1%A3.pdf` |
| `files/info.txt` | `files%2Finfo.txt` |

**客户端处理**: 大多数 S3 客户端（boto3、rusty-s3）会自动处理 URL 编码。

**服务端处理**: 
- 接收请求时，自动 URL 解码键
- 在 RocksDB 中存储原始解码后的键
- 返回响应时，URL 编码键

---

## 状态码

| 状态码 | 说明 |
|--------|------|
| 200 | 成功 |
| 204 | No Content（删除成功） |
| 400 | 请求参数错误 |
| 403 | 签名验证失败 |
| 404 | 资源不存在 |
| 500 | 服务器内部错误 |

---

## 启动服务

### 开发模式

```bash
# 构建项目
cargo build --release

# 初始化数据库（首次使用）
./target/release/laoflchDB-rust init

# 启动服务
./target/release/laoflchDB-rust start
```

服务将同时启动:
- **gRPC 服务**: `0.0.0.0:19777`
- **REST 服务**: `0.0.0.0:8080`
- **S3 服务**: `0.0.0.0:9000`

### Docker 部署

```bash
# 构建镜像
cargo docker build

# 启动容器
cargo docker start
```

---

## 与 REST API 的区别

| 特性 | S3 API (/:9000) | REST API (/:8080) |
|------|-----------------|-------------------|
| 认证方式 | AWS Signature V4 | Bearer Token |
| 响应格式 | XML | JSON |
| 端点前缀 | 无 | `/api/v1/object-store` |
| 支持预签名 URL | ✅ 是 | ❌ 否 |
| 与标准 S3 客户端兼容 | ✅ 是 | ❌ 否 |
