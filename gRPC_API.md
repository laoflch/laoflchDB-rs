# LaoflchDB gRPC API 文档

## 基础信息

- **服务地址**: `localhost:29777`
- **协议**: gRPC (HTTP/2)
- **语言**: Protocol Buffers 3

## 服务定义

```protobuf
service LaoflchDb {
  // KV 操作
  rpc Get(GetRequest) returns (GetResponse);
  rpc Put(PutRequest) returns (PutResponse);
  rpc Delete(DeleteRequest) returns (DeleteResponse);
  
  // 表管理
  rpc CreateTable(CreateTableRequest) returns (CreateTableResponse);
  rpc DropTable(DropTableRequest) returns (DropTableResponse);
  rpc ListTables(ListTablesRequest) returns (ListTablesResponse);
  rpc ListTableCols(ListTableColsRequest) returns (ListTableColsResponse);
  
  // 行操作
  rpc AddRow(AddRowRequest) returns (AddRowResponse);
  rpc GetRow(GetRowRequest) returns (GetRowResponse);
  rpc DeleteRow(DeleteRowRequest) returns (DeleteRowResponse);
  rpc UpdateRow(UpdateRowRequest) returns (UpdateRowResponse);
  
  // 元数据查询
  rpc GetAllMeta(GetAllMetaRequest) returns (GetAllMetaResponse);
  rpc GetSchemaInfo(GetSchemaInfoRequest) returns (GetSchemaInfoResponse);
  rpc ListSchemas(ListSchemasRequest) returns (ListSchemasResponse);
  
  // 查询操作
  rpc Query(QueryRequest) returns (QueryResponse);
  rpc SqlQuery(SqlQueryRequest) returns (SqlQueryResponse);
}
```

---

## 消息类型

### 1. KV 操作

#### GetRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| schema | string | 是 | 数据库 schema 名称 |
| table | string | 是 | 表名 |
| key | bytes | 是 | 键值 |

#### GetResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| value | bytes | 返回的值 |
| message | string | 错误信息 |

#### PutRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| schema | string | 是 | 数据库 schema 名称 |
| table | string | 是 | 表名 |
| key | bytes | 是 | 键值 |
| value | bytes | 是 | 值 |

#### PutResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| message | string | 错误信息 |

#### DeleteRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| schema | string | 是 | 数据库 schema 名称 |
| table | string | 是 | 表名 |
| key | bytes | 是 | 键值 |

#### DeleteResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| message | string | 错误信息 |

---

### 2. 表管理

#### CreateTableRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| schema | string | 是 | 数据库 schema 名称 |
| table_name | string | 是 | 表名 |
| columns | repeated ColumnDef | 是 | 列定义列表 |

#### ColumnDef

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| name | string | 是 | 列名 |
| column_type | int32 | 是 | 列类型（1=Int64, 2=String, 3=Bytes, 4=Float, 5=List, 6=Image） |

#### CreateTableResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| table_id | uint64 | 表 ID |
| message | string | 错误信息 |

#### DropTableRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| schema | string | 是 | 数据库 schema 名称 |
| table_name | string | 是 | 表名 |

#### DropTableResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| message | string | 错误信息 |

#### ListTablesRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| schema | string | 是 | 数据库 schema 名称 |

#### ListTablesResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| tables | repeated string | 表名列表 |
| message | string | 错误信息 |

#### ListTableColsRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| schema | string | 是 | 数据库 schema 名称 |
| table_name | string | 是 | 表名 |

#### ListTableColsResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| columns | repeated ColumnMeta | 列元数据列表 |
| message | string | 错误信息 |

#### ColumnMeta

| 字段 | 类型 | 说明 |
|------|------|------|
| table_id | uint64 | 表 ID |
| column_id | uint64 | 列 ID |
| column_name | string | 列名 |
| column_type | int32 | 列类型 |

---

### 3. 行操作

#### AddRowRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| schema | string | 是 | 数据库 schema 名称 |
| table_name | string | 是 | 表名 |
| row | Row | 是 | 行数据 |

#### AddRowResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| row_id | uint64 | 行 ID（Snowflake ID） |
| message | string | 错误信息 |

#### GetRowRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| schema | string | 是 | 数据库 schema 名称 |
| table_name | string | 是 | 表名 |
| row_id | uint64 | 是 | 行 ID |

#### GetRowResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| row | Row | 行数据 |
| message | string | 错误信息 |

#### DeleteRowRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| schema | string | 是 | 数据库 schema 名称 |
| table_name | string | 是 | 表名 |
| row_id | uint64 | 是 | 行 ID |

#### DeleteRowResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| message | string | 错误信息 |

#### UpdateRowRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| schema | string | 是 | 数据库 schema 名称 |
| table_name | string | 是 | 表名 |
| row_id | uint64 | 是 | 行 ID |
| row | Row | 是 | 更新后的行数据 |

#### UpdateRowResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| message | string | 错误信息 |

---

### 4. 元数据查询

#### GetAllMetaRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| schema | string | 是 | 数据库 schema 名称 |

#### GetAllMetaResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| meta_json | string | 所有元数据（JSON 格式） |
| message | string | 错误信息 |

#### GetSchemaInfoRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| schema | string | 是 | 数据库 schema 名称 |

#### GetSchemaInfoResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| info_json | string | Schema 信息（JSON 格式） |
| message | string | 错误信息 |

#### ListSchemasRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| (无) | - | - | 无参数 |

#### ListSchemasResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| schemas | repeated string | Schema 名称列表 |
| message | string | 错误信息 |

---

#### QueryRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| schema | string | 是 | 数据库 schema 名称 |
| table_filters | repeated TableFilter | 是 | 表过滤器列表（AND 关系） |
| limit | uint32 | 否 | 返回结果数量限制 |
| offset | uint32 | 否 | 跳过的结果数量 |

#### TableFilter

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| table_name | string | 是 | 表名 |
| column_filters | repeated ColumnFilter | 是 | 列过滤器列表（AND 关系） |

#### ColumnFilter

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| column_name | string | 是 | 列名 |
| conditions | repeated ColumnFilterCondition | 是 | 过滤条件列表（OR 关系） |

#### ColumnFilterCondition

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| op | FilterOperator | 是 | 操作符 |
| value | Field | 是 | 比较值 |
| values | repeated Field | 否 | 比较值列表（用于 IN 操作） |

#### FilterOperator（枚举）

| 值 | 枚举值 | 说明 |
|------|--------|------|
| FILTER_OPERATOR_UNSPECIFIED | 0 | 未指定 |
| FILTER_OPERATOR_EQ | 1 | 等于 |
| FILTER_OPERATOR_NEQ | 2 | 不等于 |
| FILTER_OPERATOR_GT | 3 | 大于 |
| FILTER_OPERATOR_GTE | 4 | 大于等于 |
| FILTER_OPERATOR_LT | 5 | 小于 |
| FILTER_OPERATOR_LTE | 6 | 小于等于 |
| FILTER_OPERATOR_IN | 7 | 在列表中 |
| FILTER_OPERATOR_NOT_IN | 8 | 不在列表中 |
| FILTER_OPERATOR_LIKE | 9 | 模糊匹配 |
| FILTER_OPERATOR_IS_NULL | 10 | 为空 |
| FILTER_OPERATOR_IS_NOT_NULL | 11 | 不为空 |

#### Field

| 字段 | 类型 | 说明 |
|------|------|------|
| string_value | StringValue | 字符串值 |
| integer_value | IntegerValue | 整数值 |
| bytes_value | BytesValue | 字节值 |
| float_value | FloatValue | 浮点值 |
| list_value | ListValue | 列表值 |
| image_value | ImageValue | 图像值 |

#### QueryResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| rows | repeated QueryRow | 查询结果行 |
| message | string | 错误信息 |

#### QueryRow

| 字段 | 类型 | 说明 |
|------|------|------|
| table_name | string | 表名 |
| row_id | uint64 | 行 ID |
| row | Row | 行数据 |

#### SqlQueryRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| schema | string | 是 | 数据库 schema 名称 |
| sql | string | 是 | SQL 查询语句 |

#### SqlQueryResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| columns | repeated string | 列名列表 |
| rows | repeated QueryRow | 查询结果行 |
| message | string | 错误信息 |

---

### 6. 通用类型

#### Row

| 字段 | 类型 | 说明 |
|------|------|------|
| row_type | RowType | 行类型（0=NORMAL, 1=RAW） |
| version | uint32 | 版本号 |
| data | repeated bytes | 列数据列表 |

#### RowType（枚举）

| 值 | 枚举值 | 说明 |
|------|--------|------|
| ROW_TYPE_NORMAL | 0 | 普通行 |
| ROW_TYPE_RAW | 1 | 原始行 |

---

## 使用示例

### Python 示例

```python
import grpc
import rpc_pb2
import rpc_pb2_grpc

# 建立连接
channel = grpc.insecure_channel("localhost:29777")
stub = rpc_pb2_grpc.LaoflchDbStub(channel)

# 1. 创建表
resp = stub.CreateTable(rpc_pb2.CreateTableRequest(
    schema="sys",
    table_name="users",
    columns=[
        rpc_pb2.ColumnDef(name="id", column_type=1),
        rpc_pb2.ColumnDef(name="name", column_type=2),
        rpc_pb2.ColumnDef(name="email", column_type=2),
    ]
))
print(f"Create table: {resp.success}")

# 2. 插入数据
resp = stub.Put(rpc_pb2.PutRequest(
    schema="sys",
    table="users",
    key=b"user_001",
    value=b'{"id":1,"name":"Alice","email":"alice@example.com"}'
))
print(f"Put: {resp.success}")

# 3. 读取数据
resp = stub.Get(rpc_pb2.GetRequest(
    schema="sys",
    table="users",
    key=b"user_001"
))
print(f"Get: {resp.success}, value={resp.value.decode()}")

# 4. 查询数据（CNF 表达式）
resp = stub.Query(rpc_pb2.QueryRequest(
    schema="sys",
    table_filters=[
        rpc_pb2.TableFilter(
            table_name="users",
            column_filters=[
                rpc_pb2.ColumnFilter(
                    column_name="id",
                    conditions=[
                        rpc_pb2.ColumnFilterCondition(
                            op=rpc_pb2.FILTER_OPERATOR_GT,
                            value=rpc_pb2.Field(integer_value=rpc_pb2.IntegerValue(value=0))
                        )
                    ]
                )
            ]
        )
    ],
    limit=10,
    offset=0
))
print(f"Query: {resp.success}, rows={len(resp.rows)}")

# 5. 删除数据
resp = stub.Delete(rpc_pb2.DeleteRequest(
    schema="sys",
    table="users",
    key=b"user_001"
))
print(f"Delete: {resp.success}")

# 6. 删除表
resp = stub.DropTable(rpc_pb2.DropTableRequest(
    schema="sys",
    table_name="users"
))
print(f"Drop table: {resp.success}")
```

### Go 示例

```go
package main

import (
    "context"
    "fmt"
    "log"

    "google.golang.org/grpc"
    pb "path/to/proto"
)

func main() {
    conn, err := grpc.Dial("localhost:29777", grpc.WithInsecure())
    if err != nil {
        log.Fatalf("did not connect: %v", err)
    }
    defer conn.Close()
    
    client := pb.NewLaoflchDbClient(conn)

    // 创建表
    resp, err := client.CreateTable(context.Background(), &pb.CreateTableRequest{
        Schema:    "sys",
        TableName: "users",
        Columns: []*pb.ColumnDef{
            {Name: "id", ColumnType: 1},
            {Name: "name", ColumnType: 2},
        },
    })
    fmt.Printf("Create table: %v\n", resp.Success)
}
```

---

## 启动服务

```bash
# 构建项目
cargo build --release

# 启动服务
./target/release/laoflchDB-rust start

# Docker 部署
cargo docker deploy
```

服务启动后监听：
- **gRPC**: `0.0.0.0:29777`
- **REST**: `0.0.0.0:38080`

---

## 配置文件

```yaml
access_protocols:
  - protocol: grpc
    enabled: true
    addr: 0.0.0.0:29777
    service_id: grpc_admin

  - protocol: rest
    enabled: true
    addr: 0.0.0.0:38080
    service_id: rest_admin

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
```

---

## 自动测试

```bash
# 运行 gRPC 测试
python3 tests_python/test_final.py

# 运行完整测试（REST + gRPC）
cargo auto-test prod
```

---

## 错误码

| gRPC 状态码 | 说明 |
|-------------|------|
| OK (0) | 成功 |
| INTERNAL (13) | 服务器内部错误 |
| INVALID_ARGUMENT (3) | 参数错误 |
| PERMISSION_DENIED (7) | 权限不足 |