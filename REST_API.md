# LaoflchDB REST API 文档

## 基础信息

- **Base URL**: `http://localhost:8080`
- **Content-Type**: `application/json`
- **gRPC 端口**: `19777`
- **版本**: v0.1.4

## 认证机制

LaoflchDB 使用 Token 认证机制。所有 API 请求（除登录、登出和健康检查外）都需要在请求头中携带有效的认证 Token。

### 获取 Token

通过登录接口获取 Token：

```bash
curl -X POST http://localhost:38080/api/v1/login \
  -H "Content-Type: application/json" \
  -d '{"username": "admin", "password": "admin123"}'
```

### 使用 Token

获取 Token 后，在请求头中携带 `Authorization` 头：

```bash
curl -X GET "http://localhost:38080/api/v1/get?schema=sys&table=user&key=1" \
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

## 使用示例

### cURL 示例

```bash
# 1. 健康检查
curl http://localhost:38080/health

# 2. 创建表
curl -X POST http://localhost:38080/api/v1/tables \
  -H "Content-Type: application/json" \
  -d '{"schema":"sys","table_name":"my_table","columns":[{"name":"id","column_type":"Int64"},{"name":"name","column_type":"String"}]}'

# 3. 列出所有表
curl http://localhost:38080/api/v1/schemas/sys/tables

# 4. 获取表元数据
curl http://localhost:38080/api/v1/schemas/sys/tables/my_table

# 5. 插入数据
curl -X POST http://localhost:38080/api/v1/put \
  -H "Content-Type: application/json" \
  -d '{"schema":"sys","table":"my_table","key":"user1","value":"{\"id\":1,\"name\":\"John Doe\"}"}'

# 6. 读取数据
curl "http://localhost:38080/api/v1/get?schema=sys&table=my_table&key=user1"

# 7. 删除数据
curl -X POST http://localhost:38080/api/v1/delete \
  -H "Content-Type: application/json" \
  -d '{"schema":"sys","table":"my_table","key":"user1"}'

# 8. 删除表
curl -X DELETE http://localhost:38080/api/v1/schemas/sys/tables/my_table

# 9. 查询数据
curl -X POST http://localhost:38080/api/v1/query \
  -H "Content-Type: application/json" \
  -d '{"schema":"sys","table_filters":[{"table_name":"my_table","column_filters":[{"column_name":"id","conditions":[{"operator":"GREATER_THAN","value":"5"}]}]},"limit":10,"offset":0}'
```

### Python 示例

```python
import requests
import json

BASE_URL = "http://localhost:38080"

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
- **gRPC 服务**: `0.0.0.0:29777`
- **REST 服务**: `0.0.0.0:38080`

---

## 配置文件

`laoflchdb.yaml`:

```yaml
# 数据库配置
db_path: ./laoflch_db_data
log_level: info

# 默认权限策略: allow 或 deny
default_policy: allow

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