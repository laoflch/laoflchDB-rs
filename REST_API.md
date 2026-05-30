# LaoflchDB REST API 文档

## 基础信息

- **Base URL**: `http://localhost:8080`
- **Content-Type**: `application/json`

## API 端点

### 1. 健康检查

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

### 3. 列出所有表

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

### 4. 获取表元数据

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

### 5. 插入数据

**端点**: `POST /api/v1/put`

**请求体**:
```json
{
  "schema": "sys",
  "table": "test_table",
  "key": "key1",
  "value": "value1"
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

### 6. 读取数据

**端点**: `GET /api/v1/get?schema={schema}&table={table}&key={key}`

**响应示例**:
```json
{
  "success": true,
  "message": "",
  "data": {
    "value": [118, 97, 108, 117, 101, 49]
  }
}
```

**注意**: `value` 是字节数组，如果存储的是字符串，转换为UTF-8即可。

---

### 7. 删除数据

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
  -d '{"schema":"sys","table":"my_table","key":"user1","value":"John Doe"}'

# 6. 读取数据
curl "http://localhost:8080/api/v1/get?schema=sys&table=my_table&key=user1"

# 7. 删除数据
curl -X POST http://localhost:8080/api/v1/delete \
  -H "Content-Type: application/json" \
  -d '{"schema":"sys","table":"my_table","key":"user1"}'
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
        "value": "Alice"
    }
    resp = requests.post(f"{BASE_URL}/api/v1/put", json=data)
    print(resp.json())

# 读取数据
def get_data():
    resp = requests.get(f"{BASE_URL}/api/v1/get",
                        params={"schema": "sys", "table": "users", "key": "1"})
    result = resp.json()
    if result["success"] and result["data"]["value"]:
        # 将字节数组转换为字符串
        value = bytes(result["data"]["value"]).decode("utf-8")
        print(f"Value: {value}")
```

---

## 启动服务

```bash
# 初始化数据库（首次使用）
./target/debug/laoflchDB-rust -c laoflchdb.yaml init

# 启动服务
./target/debug/laoflchDB-rust -c laoflchdb.yaml start
```

服务将同时启动:
- **gRPC 服务**: `127.0.0.1:50051`
- **REST 服务**: `127.0.0.1:8080`

---

## 配置文件

`laoflchdb.yaml`:

```yaml
db_path: ./laoflch_db_data
addr: 127.0.0.1:50051
log_level: info

access_protocols:
  - protocol: grpc
    enabled: true
    addr: 127.0.0.1:50051

  - protocol: rest
    enabled: true
    addr: 127.0.0.1:8080
```
