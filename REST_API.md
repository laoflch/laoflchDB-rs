# LaoflchDB REST API 文档

## 基础信息

- **Base URL**: `http://localhost:8080`
- **Content-Type**: `application/json`
- **gRPC 端口**: `19777`
- **版本**: v0.1.9

## 认证机制

LaoflchDB 使用 Token 认证机制。所有 API 请求（除登录、登出和健康检查外）都需要在请求头中携带有效的认证 Token。

### 获取 Token

通过登录接口获取 Token：

```bash
curl -X POST http://localhost:8080/api/v1/login \
  -H "Content-Type: application/json" \
  -d '{"username": "admin", "password": "admin123"}'
```

### 使用 Token

获取 Token 后，在请求头中携带 `Authorization` 头：

```bash
curl -X GET "http://localhost:8080/api/v1/get?schema=sys&table=user&key=1" \
  -H "Authorization: Bearer <your_token>"
```

## API 端点

### 1. 用户登录

**端点**: `POST /api/v1/login`

**请求体**:
```json
{
  "username": "admin",
  "password": "admin123"
}
```

**响应示例**:
```json
{
  "success": true,
  "message": "",
  "data": {
    "success": true,
    "message": "Login successful",
    "token": "550e8400-e29b-41d4-a716-446655440000",
    "user_id": 1,
    "username": "admin"
  }
}
```

### 2. 用户登出

**端点**: `POST /api/v1/logout`

**请求体**:
```json
{
  "token": "550e8400-e29b-41d4-a716-446655440000"
}
```

**响应示例**:
```json
{
  "success": true,
  "message": "",
  "data": "Logout successful"
}
```

### 3. 健康检查

**端点**: `GET /health`

**响应示例**:
```json
{
  "success": true,
  "message": "",
  "data": "OK"
}
```

---

### 2. 创建表

**端点**: `POST /api/v1/tables`

**请求体**:
```json
{
  "schema": "sys",
  "table_name": "test_table",
  "columns": [
    {"name": "id", "column_type": "Int64"},
    {"name": "data", "column_type": "String"}
  ]
}
```

**支持的列类型**:
- `STRING` - 字符串
- `INT64` / `INT` - 64位整数
- `BYTES` / `BINARY` - 字节数组
- `FLOAT` / `DOUBLE` - 浮点数
- `LIST` - 列表
- `IMAGE` - 图像

**响应示例**:
```json
{
  "success": true,
  "message": "",
  "data": {
    "table_id": 0
  }
}
```

---

### 3. 删除表

**端点**: `DELETE /api/v1/schemas/{schema}/tables/{table}`

**响应示例**:
```json
{
  "success": true,
  "message": "",
  "data": "OK"
}
```

---

### 4. 列出所有表

**端点**: `GET /api/v1/schemas/{schema}/tables`

**响应示例**:
```json
{
  "success": true,
  "message": "",
  "data": ["user", "test_table"]
}
```

---

### 5. 获取表元数据

**端点**: `GET /api/v1/schemas/{schema}/tables/{table}`

**响应示例**:
```json
{
  "success": true,
  "message": "",
  "data": {
    "table_id": 0,
    "table_name": "test_table",
    "column_count": 2
  }
}
```

---

### 6. 插入数据

**端点**: `POST /api/v1/put`

**请求体**:
```json
{
  "schema": "sys",
  "table": "test_table",
  "key": "key1",
  "value": "{\"id\":1,\"name\":\"Alice\"}"
}
```

**响应示例**:
```json
{
  "success": true,
  "message": "",
  "data": "OK"
}
```

---

### 7. 读取数据

**端点**: `GET /api/v1/get?schema={schema}&table={table}&key={key}`

**响应示例**:
```json
{
  "success": true,
  "message": "",
  "data": {
    "value": "{\"id\":1,\"name\":\"Alice\"}"
  }
}
```

---

### 8. 删除数据

**端点**: `POST /api/v1/delete`

**请求体**:
```json
{
  "schema": "sys",
  "table": "test_table",
  "key": "key1"
}
```

**响应示例**:
```json
{
  "success": true,
  "message": "",
  "data": "OK"
}
```

---

### 9. 查询数据 (CNF 表达式)

**端点**: `POST /api/v1/query`

**请求体**:
```json
{
  "schema": "sys",
  "table_filters": [
    {
      "table_name": "test_table",
      "column_filters": [
        {
          "column_name": "id",
          "conditions": [
            {
              "operator": "GREATER_THAN",
              "value": "10"
            }
          ]
        }
      ]
    }
  ],
  "limit": 10,
  "offset": 0
}
```

**支持的操作符**:
- `EQUALS` - 等于
- `NOT_EQUALS` - 不等于
- `GREATER_THAN` - 大于
- `GREATER_THAN_OR_EQUALS` - 大于等于
- `LESS_THAN` - 小于
- `LESS_THAN_OR_EQUALS` - 小于等于
- `CONTAINS` - 包含
- `STARTS_WITH` - 前缀匹配

**响应示例**:
```json
{
  "success": true,
  "message": "",
  "data": {
    "rows": [
      {"row_id": 11, "data": "{\"id\":11,\"name\":\"Bob\"}"},
      {"row_id": 12, "data": "{\"id\":12,\"name\":\"Charlie\"}"}
    ],
    "total_count": 2
  }
}
```

---

### 10. 全文索引操作

#### 10.1 创建索引

**端点**: `POST /api/v1/index/indices`

**请求体**:
```json
{
  "name": "test_index",
  "fields": [
    {"name": "title", "type": "TEXT", "indexed": true, "stored": true},
    {"name": "content", "type": "TEXT", "indexed": true, "stored": true},
    {"name": "category", "type": "STRING", "indexed": true, "stored": true},
    {"name": "view_count", "type": "INT", "indexed": true, "stored": true}
  ]
}
```

**字段类型**:
- `TEXT` - 文本字段（全文索引）
- `STRING` - 字符串字段（精确匹配）
- `INT` - 整数字段
- `FLOAT` - 浮点数字段

**响应示例**:
```json
{
  "success": true,
  "message": "",
  "data": {
    "index_id": 9723294756888291148
  }
}
```

#### 10.2 删除索引

**端点**: `DELETE /api/v1/index/indices/{INDEX_NAME}`

**响应示例**:
```json
{
  "success": true,
  "message": "",
  "data": "OK"
}
```

#### 10.3 列出所有索引

**端点**: `GET /api/v1/index/indices`

**响应示例**:
```json
{
  "success": true,
  "message": "",
  "data": ["test_index", "articles"]
}
```

#### 10.4 获取索引字段

**端点**: `GET /api/v1/index/indices/{INDEX_NAME}/fields`

**响应示例**:
```json
{
  "success": true,
  "message": "",
  "data": ["title", "content", "category", "view_count"]
}
```

#### 10.5 获取索引元数据

**端点**: `GET /api/v1/index/indices/{INDEX_NAME}/meta`

**响应示例**:
```json
{
  "success": true,
  "message": "",
  "data": {
    "name": "test_index",
    "columns": 4
  }
}
```

#### 10.6 获取索引统计

**端点**: `GET /api/v1/index/stats`

**响应示例**:
```json
{
  "success": true,
  "message": "",
  "data": {
    "total": 2,
    "names": ["test_index", "articles"]
  }
}
```

#### 10.7 添加文档

**端点**: `POST /api/v1/index/indices/{INDEX_NAME}/docs`

**请求体**:
```json
{
  "doc_id": "doc_001",
  "fields": {
    "title": "Introduction to Rust",
    "content": "Rust is a systems programming language...",
    "category": "Programming",
    "view_count": "1000"
  }
}
```

**说明**: `doc_id` 为可选字段，如果不提供则自动生成 Snowflake ID。

**响应示例**:
```json
{
  "success": true,
  "message": "",
  "data": {
    "doc_id": "doc_001"
  }
}
```

#### 10.8 获取文档

**端点**: `GET /api/v1/index/indices/{INDEX_NAME}/docs/{DOC_ID}`

**响应示例**:
```json
{
  "success": true,
  "message": "",
  "data": {
    "doc_id": "doc_001",
    "score": 0.0,
    "fields": {
      "title": "Introduction to Rust",
      "content": "Rust is a systems programming language...",
      "category": "Programming",
      "view_count": "1000"
    }
  }
}
```

#### 10.9 删除文档

**端点**: `DELETE /api/v1/index/indices/{INDEX_NAME}/docs/{DOC_ID}`

**响应示例**:
```json
{
  "success": true,
  "message": "",
  "data": "OK"
}
```

#### 10.10 搜索文档

**端点**: `POST /api/v1/index/indices/{INDEX_NAME}/search`

**请求体**:
```json
{
  "query": "Rust programming",
  "fields": ["title", "content"],
  "limit": 10,
  "offset": 0
}
```

**响应示例**:
```json
{
  "success": true,
  "message": "",
  "data": {
    "total_hits": 5,
    "results": [
      {
        "doc_id": "doc_001",
        "score": 0.95,
        "fields": {
          "title": "Introduction to Rust",
          "content": "Rust is a systems programming language..."
        }
      }
    ]
  }
}
```

---

### 11. 对象存储 (S3 兼容 REST API)

对象存储服务提供 **S3 兼容的 REST API**，基于 RocksDB BlobDB 实现大对象存储。所有端点挂载在 `/api/v1/object-store` 前缀下。

#### 11.1 ListBuckets - 列出所有 Bucket

**端点**: `GET /api/v1/object-store`

**响应示例**:
```json
{
  "buckets": [
    {"name": "my-bucket", "creation_date": "1720000000"},
    {"name": "photos", "creation_date": "1720001234"}
  ]
}
```

#### 11.2 CreateBucket - 创建 Bucket

**端点**: `PUT /api/v1/object-store/{bucket}`

**响应**: 成功返回 `200 OK`，空响应体。

**说明**: 重复创建同一 Bucket 是幂等操作，返回成功。

#### 11.3 DeleteBucket - 删除 Bucket

**端点**: `DELETE /api/v1/object-store/{bucket}`

**响应**: 成功返回 `204 No Content`。

#### 11.4 ListObjects - 列出 Bucket 中的对象

**端点**: `GET /api/v1/object-store/{bucket}`

**查询参数**:

| 参数 | 类型 | 说明 |
|------|------|------|
| prefix | string | 仅列出键以该前缀开头的对象 |
| delimiter | string | 目录分隔符（通常为 `/`），用于模拟目录结构 |
| max_keys | int32 | 返回对象数量上限（默认 1000） |
| marker | string | 分页起始键（从上一次 `next_marker` 继续） |

**响应示例**:
```json
{
  "bucket": "my-bucket",
  "objects": [
    {
      "key": "photos/cat.jpg",
      "size": 102400,
      "etag": "\"a1b2c3d4e5f6...\"",
      "last_modified": "1720000000",
      "content_type": "image/jpeg"
    }
  ],
  "common_prefixes": ["photos/2024/"],
  "is_truncated": false,
  "next_marker": ""
}
```

#### 11.5 PutObject - 上传对象

**端点**: `PUT /api/v1/object-store/{bucket}/{key}`

**请求头**:
- `Content-Type`: 对象的 MIME 类型（默认 `application/octet-stream`）

**请求体**: 原始二进制数据（任意字节）

**响应示例**:
```json
{"etag": "\"a1b2c3d4e5f6...\""}
```

**说明**: 每次上传都会生成新的 ETag（基于 UUID），覆盖已有对象时 ETag 会更新。支持大对象上传（基于 BlobDB，默认单文件最大 256MB）。

#### 11.6 GetObject - 下载对象

**端点**: `GET /api/v1/object-store/{bucket}/{key}`

**响应头**:
- `Content-Type`: 对象的 MIME 类型
- `Content-Length`: 对象字节数
- `ETag`: 对象的 ETag

**响应体**: 原始二进制数据

**错误**: 对象不存在时返回 `404 Not Found`。

#### 11.7 HeadObject - 获取对象元数据

**端点**: `HEAD /api/v1/object-store/{bucket}/{key}`

**响应头**:
- `Content-Type`: 对象的 MIME 类型
- `Content-Length`: 对象字节数
- `ETag`: 对象的 ETag
- `Last-Modified`: 最后修改时间（Unix 时间戳字符串）

**响应体**: 空

**错误**: 对象不存在时返回 `404 Not Found`。

#### 11.8 DeleteObject - 删除对象

**端点**: `DELETE /api/v1/object-store/{bucket}/{key}`

**响应**: 成功返回 `204 No Content`。

**说明**: 删除不存在的对象是幂等操作，始终返回成功。

#### 对象存储 cURL 示例

```bash
# 1. 列出所有 Bucket
curl http://localhost:8080/api/v1/object-store \
  -H "Authorization: Bearer <your_token>"

# 2. 创建 Bucket
curl -X PUT http://localhost:8080/api/v1/object-store/my-bucket \
  -H "Authorization: Bearer <your_token>"

# 3. 上传文件
curl -X PUT http://localhost:8080/api/v1/object-store/my-bucket/photos/cat.jpg \
  -H "Content-Type: image/jpeg" \
  -H "Authorization: Bearer <your_token>" \
  --data-binary @/path/to/cat.jpg

# 4. 下载文件
curl http://localhost:8080/api/v1/object-store/my-bucket/photos/cat.jpg \
  -H "Authorization: Bearer <your_token>" \
  -o cat.jpg

# 5. 获取对象元数据（HEAD）
curl -I http://localhost:8080/api/v1/object-store/my-bucket/photos/cat.jpg \
  -H "Authorization: Bearer <your_token>"

# 6. 列出 Bucket 中的对象
curl "http://localhost:8080/api/v1/object-store/my-bucket?prefix=photos/&delimiter=/" \
  -H "Authorization: Bearer <your_token>"

# 7. 删除对象
curl -X DELETE http://localhost:8080/api/v1/object-store/my-bucket/photos/cat.jpg \
  -H "Authorization: Bearer <your_token>"

# 8. 删除 Bucket
curl -X DELETE http://localhost:8080/api/v1/object-store/my-bucket \
  -H "Authorization: Bearer <your_token>"
```

**路由语法说明**: 本项目使用 Axum 0.7，路径参数采用 `:param` 和 `/*catch_all` 语法（而非 Axum 0.8 的 `{param}` 语法）。对象存储路径 `/api/v1/object-store/:bucket/*key` 中，`:bucket` 匹配 Bucket 名称，`*key` 匹配对象键（可包含 `/`，用于模拟目录结构）。

---

### 12. 图片服务 (ImageService REST API)

图片服务基于对象存储服务实现，提供图片上传（自动生成三种规格缩略图）和浏览功能。所有端点挂载在 `/api/v1/images` 前缀下。

#### 12.1 UploadImage - 上传图片

**端点**: `POST /api/v1/images`

**查询参数**:

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| bucket | string | 否 | Bucket 名称（为空时使用默认 bucket "images"） |
| key | string | 否 | 图片 key（为空时自动生成 UUID） |

**请求头**:
- `Content-Type`: 图片的 MIME 类型（如 `image/jpeg`、`image/png`）

**请求体**: 原始图片二进制数据（PNG/JPEG/GIF/WebP）

**响应示例**:
```json
{
  "success": true,
  "key": "photo.jpg",
  "etag": "\"a1b2c3d4...\"",
  "metadata": {
    "key": "photo.jpg",
    "content_type": "image/jpeg",
    "content_length": 102400,
    "width": 1920,
    "height": 1080,
    "etag": "\"a1b2c3d4...\"",
    "last_modified": "1720000000",
    "thumbnails": {
      "thumbnail": "photo.jpg__thumbnail.jpg",
      "small": "photo.jpg__small.jpg",
      "medium": "photo.jpg__medium.jpg"
    },
    "format": "Jpeg"
  }
}
```

**说明**: 上传时自动生成三种缩略图：
- `thumbnail`: 128x128，cover 模式（裁剪为正方形），JPEG 编码
- `small`: 最大 256x256，contain 模式（等比缩放），JPEG 编码
- `medium`: 最大 512x512，contain 模式（等比缩放），JPEG 编码

#### 12.2 ListImages - 列出图片

**端点**: `GET /api/v1/images`

**查询参数**:

| 参数 | 类型 | 说明 |
|------|------|------|
| bucket | string | Bucket 名称 |
| prefix | string | 仅列出 key 以该前缀开头的图片 |
| max_keys | int32 | 返回数量上限（默认 100） |
| marker | string | 分页起始键 |

#### 12.3 GetImage - 获取原图

**端点**: `GET /api/v1/images/{key}`

**查询参数**: `bucket`（可选）

**响应头**:
- `Content-Type`: 图片的 MIME 类型
- `Content-Length`: 图片字节数
- `ETag`: 图片的 ETag

**响应体**: 原始图片二进制数据

#### 12.4 GetImageMetadata - 获取图片元数据

**端点**: `GET /api/v1/images/{key}/meta`

**查询参数**: `bucket`（可选）

**响应**: 图片元数据 JSON（包含 width、height、format、thumbnails、user_metadata 等）

#### 12.5 GetThumbnail - 获取缩略图

**端点**: `GET /api/v1/images/{key}/thumbnails/{size}`

**路径参数**:
- `key`: 图片 key
- `size`: 缩略图规格（`thumbnail`、`small`、`medium`）

**查询参数**: `bucket`（可选）

**响应头**:
- `Content-Type`: `image/jpeg`
- `Content-Length`: 缩略图字节数
- `X-Thumbnail-Width`: 缩略图宽度
- `X-Thumbnail-Height`: 缩略图高度

**响应体**: JPEG 格式的缩略图二进制数据

**错误**: 无效的 `size` 参数返回 `400 Bad Request`，图片不存在返回 `404 Not Found`

#### 12.6 DeleteImage - 删除图片

**端点**: `DELETE /api/v1/images/{key}`

**查询参数**: `bucket`（可选）

**响应示例**:
```json
{
  "success": true,
  "deleted_keys": [
    "photo.jpg",
    "photo.jpg__thumbnail.jpg",
    "photo.jpg__small.jpg",
    "photo.jpg__medium.jpg",
    "__img_meta__photo.jpg"
  ]
}
```

**说明**: 删除图片时级联删除原图、所有缩略图和元数据（共 5 个对象）。删除不存在的图片是幂等操作。

#### 图片服务 cURL 示例

```bash
# 1. 上传图片（自动生成缩略图）
curl -X POST "http://localhost:8080/api/v1/images?bucket=my-images&key=photo.jpg" \
  -H "Content-Type: image/jpeg" \
  -H "Authorization: Bearer <your_token>" \
  --data-binary @/path/to/photo.jpg

# 2. 获取原图
curl "http://localhost:8080/api/v1/images/photo.jpg?bucket=my-images" \
  -H "Authorization: Bearer <your_token>" \
  -o photo.jpg

# 3. 获取 thumbnail 缩略图（128x128）
curl "http://localhost:8080/api/v1/images/photo.jpg/thumbnails/thumbnail?bucket=my-images" \
  -H "Authorization: Bearer <your_token>" \
  -o thumb.jpg

# 4. 获取 small 缩略图（最大 256x256）
curl "http://localhost:8080/api/v1/images/photo.jpg/thumbnails/small?bucket=my-images" \
  -H "Authorization: Bearer <your_token>" \
  -o small.jpg

# 5. 获取 medium 缩略图（最大 512x512）
curl "http://localhost:8080/api/v1/images/photo.jpg/thumbnails/medium?bucket=my-images" \
  -H "Authorization: Bearer <your_token>" \
  -o medium.jpg

# 6. 获取图片元数据
curl "http://localhost:8080/api/v1/images/photo.jpg/meta?bucket=my-images" \
  -H "Authorization: Bearer <your_token>"

# 7. 列出图片
curl "http://localhost:8080/api/v1/images?bucket=my-images&max_keys=100" \
  -H "Authorization: Bearer <your_token>"

# 8. 带前缀列出图片
curl "http://localhost:8080/api/v1/images?bucket=my-images&prefix=album/&max_keys=100" \
  -H "Authorization: Bearer <your_token>"

# 9. 删除图片（级联删除原图+缩略图+元数据）
curl -X DELETE "http://localhost:8080/api/v1/images/photo.jpg?bucket=my-images" \
  -H "Authorization: Bearer <your_token>"
```

---

## 使用示例

### cURL 示例

```bash
# 1. 健康检查
curl http://localhost:8080/health

# 2. 创建表
curl -X POST http://localhost:8080/api/v1/tables \
  -H "Content-Type: application/json" \
  -d '{"schema":"sys","table_name":"my_table","columns":[{"name":"id","column_type":"Int64"},{"name":"name","column_type":"String"}]}'

# 3. 列出所有表
curl http://localhost:8080/api/v1/schemas/sys/tables

# 4. 获取表元数据
curl http://localhost:8080/api/v1/schemas/sys/tables/my_table

# 5. 插入数据
curl -X POST http://localhost:8080/api/v1/put \
  -H "Content-Type: application/json" \
  -d '{"schema":"sys","table":"my_table","key":"user1","value":"{\"id\":1,\"name\":\"John Doe\"}"}'

# 6. 读取数据
curl "http://localhost:8080/api/v1/get?schema=sys&table=my_table&key=user1"

# 7. 删除数据
curl -X POST http://localhost:8080/api/v1/delete \
  -H "Content-Type: application/json" \
  -d '{"schema":"sys","table":"my_table","key":"user1"}'

# 8. 删除表
curl -X DELETE http://localhost:8080/api/v1/schemas/sys/tables/my_table

# 9. 查询数据
curl -X POST http://localhost:8080/api/v1/query \
  -H "Content-Type: application/json" \
  -d '{"schema":"sys","table_filters":[{"table_name":"my_table","column_filters":[{"column_name":"id","conditions":[{"operator":"GREATER_THAN","value":"5"}]}]},"limit":10,"offset":0}'
```

### Python 示例

```python
import requests
import json

BASE_URL = "http://localhost:8080"

# 健康检查
def health_check():
    resp = requests.get(f"{BASE_URL}/health")
    print(resp.json())

# 创建表
def create_table():
    data = {
        "schema": "sys",
        "table_name": "users",
        "columns": [
            {"name": "id", "column_type": "Int64"},
            {"name": "name", "column_type": "String"}
        ]
    }
    resp = requests.post(f"{BASE_URL}/api/v1/tables", json=data)
    print(resp.json())

# 插入数据
def put_data():
    data = {
        "schema": "sys",
        "table": "users",
        "key": "1",
        "value": json.dumps({"id": 1, "name": "Alice"})
    }
    resp = requests.post(f"{BASE_URL}/api/v1/put", json=data)
    print(resp.json())

# 读取数据
def get_data():
    resp = requests.get(f"{BASE_URL}/api/v1/get",
                        params={"schema": "sys", "table": "users", "key": "1"})
    result = resp.json()
    print(result)

# 查询数据
def query_data():
    data = {
        "schema": "sys",
        "table_filters": [
            {
                "table_name": "users",
                "column_filters": [
                    {
                        "column_name": "id",
                        "conditions": [
                            {"operator": "GREATER_THAN", "value": "0"}
                        ]
                    }
                ]
            }
        ],
        "limit": 10,
        "offset": 0
    }
    resp = requests.post(f"{BASE_URL}/api/v1/query", json=data)
    print(resp.json())

# SQL 查询
def sql_query():
    headers = {"Authorization": "Bearer <your_token>"}
    data = {
        "schema": "sys",
        "sql": "SELECT * FROM users WHERE age > 18 LIMIT 10"
    }
    resp = requests.post(f"{BASE_URL}/api/v1/sql_query", json=data, headers=headers)
    print(resp.json())

# 跨 Schema JOIN 查询
def cross_schema_join():
    headers = {"Authorization": "Bearer <your_token>"}
    data = {
        "schema": "sales",
        "sql": """
            SELECT sales.orders.order_id, inventory.products.product_name 
            FROM sales.orders 
            JOIN inventory.products ON sales.orders.product_id = inventory.products.product_id
            LIMIT 10
        """
    }
    resp = requests.post(f"{BASE_URL}/api/v1/sql_query", json=data, headers=headers)
    print(resp.json())

# 列出所有 Schema
def list_schemas():
    resp = requests.get(f"{BASE_URL}/api/v1/schemas")
    print(resp.json())
```

---

## 启动服务

### 开发模式

```bash
# 构建项目
cargo build --release

# 初始化数据库（首次使用，带示例数据）
./target/release/laoflchDB-rust init --example

# 启动服务
./target/release/laoflchDB-rust start
```

### Docker 部署

```bash
# 构建镜像
cargo docker build

# 启动容器
cargo docker start

# 完整部署（构建 + 启动）
cargo docker deploy
```

服务将同时启动:
- **gRPC 服务**: `0.0.0.0:19777`
- **REST 服务**: `0.0.0.0:8080`

---

## 配置文件

`laoflchdb.yaml`:

```yaml
# 数据库配置
db_path: ./laoflch_db_data
index_path: ./laoflch_db_index
model_path: ./laoflch_db_model
log_level: info

# 默认权限策略: allow 或 deny
default_policy: allow

# 运行时模式: multi_thread 或 single_thread
runtime_mode: multi_thread

# 向量化服务配置
vector_service:
  enabled: true
  auto_load: true
  load_models: ["bge-small-zh-v1.5", "bge-m3", "jina-clip-v2", "siglip2"]

# 嵌入向量索引服务配置
embedding_index:
  enabled: true
  dim: 512
  m: 32
  ef_construction: 200
  ef_search: 50
  max_elements: 1000000
  kv_db_path: ./laoflch_hnsw_data
  snapshot_path: ./laoflch_hnsw_snapshots

# 访问协议配置
access_protocols:
  - protocol: grpc
    enabled: true
    addr: 0.0.0.0:19777
    service_id: grpc_admin

  - protocol: rest
    enabled: true
    addr: 0.0.0.0:8080
    service_id: rest_admin

# 权限配置
permissions:
  - service_id: grpc_admin
    default_policy: allow
    allowed_actions:
      - get
      - put
      - delete
      - create_table
      - drop_table
      - list_tables
      - list_table_cols
      - add_row
      - get_row
      - update_row
      - delete_row
      - get_all_meta
      - get_schema_info
      - get_table_meta
      - query

  - service_id: rest_admin
    default_policy: allow
    allowed_actions:
      - get
      - put
      - delete
      - create_table
      - drop_table
      - list_tables
      - list_table_cols
      - add_row
      - get_row
      - update_row
      - delete_row
      - get_all_meta
      - get_schema_info
      - get_table_meta
      - query
```

---

## 自动测试

```bash
# 测试本地环境
cargo auto-test local

# 测试生产环境（Docker）
cargo auto-test prod
```

---

## 错误响应格式

```json
{
  "success": false,
  "message": "Error description",
  "data": null
}
```

## 状态码

| 状态码 | 说明 |
|--------|------|
| 200 | 成功 |
| 400 | 请求参数错误 |
| 401 | 未授权（无效或缺失的 Token） |
| 403 | 权限不足 |
| 500 | 服务器内部错误 |