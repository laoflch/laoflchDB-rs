# LaoflchDB gRPC API 文档

## 基础信息

- **服务地址**: `localhost:19777`
- **协议**: gRPC (HTTP/2)
- **语言**: Protocol Buffers 3
- **版本**: v0.1.7

## 认证机制

LaoflchDB 使用 Token 认证机制。所有 API 请求（除登录、登出外）都需要在请求元数据中携带有效的认证 Token。

**获取 Token**:
```protobuf
rpc Login(LoginRequest) returns (LoginResponse);
```

**使用 Token**:
在 gRPC 请求的元数据中添加 `authorization` 头：
```
authorization: Bearer <your_token>
```

**默认用户**:
- 用户名: `admin`
- 密码: `laoflchdb`
- 数据库初始化时自动创建

---

## 服务定义

### 1. LaoflchDb 主服务

```protobuf
service LaoflchDb {
  // 用户认证
  rpc Login(LoginRequest) returns (LoginResponse);
  rpc Logout(LogoutRequest) returns (LogoutResponse);
  
  // KV 操作
  rpc Get(GetRequest) returns (GetResponse);
  rpc Put(PutRequest) returns (PutResponse);
  rpc Delete(DeleteRequest) returns (DeleteResponse);
  
  // 表管理
  rpc CreateTable(CreateTableRequest) returns (CreateTableResponse);
  rpc DropTable(DropTableRequest) returns (DropTableResponse);
  rpc ListTables(ListTablesRequest) returns (ListTablesResponse);
  rpc ListTableCols(ListTableColsRequest) returns (ListTableColsResponse);
  rpc UpdateTableComment(UpdateTableCommentRequest) returns (UpdateTableCommentResponse);
  rpc UpdateColumnComment(UpdateColumnCommentRequest) returns (UpdateColumnCommentResponse);
  
  // 行操作
  rpc AddRow(AddRowRequest) returns (AddRowResponse);
  rpc GetRow(GetRowRequest) returns (GetRowResponse);
  rpc DeleteRow(DeleteRowRequest) returns (DeleteRowResponse);
  rpc UpdateRow(UpdateRowRequest) returns (UpdateRowResponse);
  
  // 元数据查询
  rpc GetAllMeta(GetAllMetaRequest) returns (GetAllMetaResponse);
  rpc GetSchemaInfo(GetSchemaInfoRequest) returns (GetSchemaInfoResponse);
  rpc ListSchemas(ListSchemasRequest) returns (ListSchemasResponse);
  rpc GetTableMeta(GetTableMetaRequest) returns (GetTableMetaResponse);
  rpc GetVersion(GetVersionRequest) returns (GetVersionResponse);
  
  // 查询操作
  rpc Query(QueryRequest) returns (QueryResponse);
  rpc SqlQuery(SqlQueryRequest) returns (SqlQueryResponse);
  rpc RefreshTables(RefreshTablesRequest) returns (RefreshTablesResponse);
  
  // 全文索引操作
  rpc CreateIndex(CreateIndexRequest) returns (CreateIndexResponse);
  rpc DropIndex(DropIndexRequest) returns (DropIndexResponse);
  rpc ListIndices(ListIndicesRequest) returns (ListIndicesResponse);
  rpc GetIndexFields(GetIndexFieldsRequest) returns (GetIndexFieldsResponse);
  rpc GetIndexMeta(GetIndexMetaRequest) returns (GetIndexMetaResponse);
  rpc GetIndexStats(GetIndexStatsRequest) returns (GetIndexStatsResponse);
  rpc AddDocument(AddDocumentRequest) returns (AddDocumentResponse);
  rpc GetDocument(GetDocumentRequest) returns (GetDocumentResponse);
  rpc DeleteDocument(DeleteDocumentRequest) returns (DeleteDocumentResponse);
  rpc SearchIndex(SearchIndexRequest) returns (SearchIndexResponse);
}
```

### 2. VectorService 向量化服务

```protobuf
service VectorService {
  rpc CreateEmbedding(EmbeddingRequest) returns (EmbeddingResponse);
  rpc ComputeSimilarity(SimilarityRequest) returns (SimilarityResponse);
  rpc GetModelInfo(ModelInfoRequest) returns (ModelInfoResponse);
  rpc ListModels(ListModelsRequest) returns (ListModelsResponse);
  rpc LoadModel(LoadModelRequest) returns (LoadModelResponse);
  rpc UnloadModel(UnloadModelRequest) returns (UnloadModelResponse);
  rpc ListLoadableModels(ListLoadableModelsRequest) returns (ListLoadableModelsResponse);
}
```

---

## 消息类型

### 1. 用户认证

#### LoginRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| username | string | 是 | 用户名 |
| password | string | 是 | 密码 |

#### LoginResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 登录是否成功 |
| message | string | 提示信息 |
| token | string | 登录成功后返回的认证 Token |
| user_id | int64 | 用户 ID |
| username | string | 用户名 |

#### LogoutRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| token | string | 是 | 要撤销的 Token |

#### LogoutResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 登出是否成功 |
| message | string | 提示信息 |

---

### 2. KV 操作

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

### 3. 表管理

#### CreateTableRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| schema | string | 是 | 数据库 schema 名称 |
| table_name | string | 是 | 表名 |
| columns | repeated ColumnDef | 是 | 列定义列表 |
| comment | string | 否 | 表注释 |

#### ColumnDef

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| name | string | 是 | 列名 |
| column_type | int32 | 是 | 列类型（0=String, 1=Int64, 2=Bytes, 3=Float, 4=List, 5=Image） |
| comment | string | 否 | 列注释 |

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
| comment | string | 列注释 |

#### UpdateTableCommentRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| schema | string | 是 | 数据库 schema 名称 |
| table_name | string | 是 | 表名 |
| comment | string | 是 | 新的表注释 |

#### UpdateTableCommentResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| message | string | 操作结果消息 |

#### UpdateColumnCommentRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| schema | string | 是 | 数据库 schema 名称 |
| table_name | string | 是 | 表名 |
| column_name | string | 是 | 字段名 |
| comment | string | 是 | 新的字段注释 |

#### UpdateColumnCommentResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| message | string | 操作结果消息 |

---

### 4. 行操作

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

### 5. 元数据查询

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

#### GetTableMetaRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| schema | string | 是 | 数据库 schema 名称 |
| table_name | string | 是 | 表名 |

#### GetTableMetaResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| table_id | uint64 | 表 ID |
| table_name | string | 表名 |
| column_count | uint32 | 字段数量 |
| message | string | 错误信息 |

#### GetVersionRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| (无) | - | - | 无参数 |

#### GetVersionResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| version | string | 版本号 |
| build_info | string | 构建信息（Git commit hash） |
| message | string | 错误信息 |

---

### 6. 查询操作

#### QueryRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| schema | string | 是 | 数据库 schema 名称 |
| table_filters | repeated TableFilter | 是 | 表过滤器列表（AND 关系） |
| limit | uint32 | 否 | 返回结果数量限制 |
| offset | uint32 | 否 | 跳过的结果数量 |
| projected_columns | repeated string | 否 | 投影列名列表（不填则返回所有列） |

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
| schema | string | 是 | 数据库 schema 名称（作为默认 schema） |
| sql | string | 是 | SQL 查询语句（支持跨 schema JOIN） |

#### SqlQueryResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| columns | repeated string | 列名列表 |
| rows | repeated SqlQueryResultRow | SQL 查询结果行 |
| message | string | 错误信息 |

#### SqlQueryResultRow

| 字段 | 类型 | 说明 |
|------|------|------|
| values | repeated SqlField | 行数据值列表 |

#### SqlField

| 字段 | 类型 | 说明 |
|------|------|------|
| string_value | string | 字符串值 |
| int64_value | int64 | 64 位整数值 |
| float_value | double | 浮点数值 |
| bytes_value | bytes | 字节值 |
| bool_value | bool | 布尔值 |

#### RefreshTablesRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| schema | string | 否 | Schema 名称（可选，不填则刷新所有可用表） |

#### RefreshTablesResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| tables | repeated string | 刷新的表名列表 |
| message | string | 错误信息 |

##### SQL 查询支持

**单表查询**：
```sql
SELECT * FROM users WHERE age > 18 LIMIT 10
```

**跨 Schema JOIN**：
```sql
-- 跨 schema INNER JOIN
SELECT sales.orders.order_id, inventory.products.product_name 
FROM sales.orders 
JOIN inventory.products ON sales.orders.product_id = inventory.products.product_id;

-- 跨 schema LEFT JOIN
SELECT sales.orders.order_id, inventory.products.product_name 
FROM sales.orders 
LEFT JOIN inventory.products ON sales.orders.product_id = inventory.products.product_id;

-- 三表跨 schema JOIN
SELECT 
    sales.customers.customer_name, 
    sales.orders.order_id, 
    inventory.products.product_name
FROM sales.customers 
JOIN sales.orders ON sales.customers.customer_id = sales.orders.customer_id 
JOIN inventory.products ON sales.orders.product_id = inventory.products.product_id;
```

**SQL 语法支持**：

| 功能 | 说明 |
|------|------|
| SELECT | 基础查询 |
| WHERE | 条件过滤 |
| ORDER BY | 排序 |
| LIMIT/OFFSET | 分页 |
| GROUP BY | 分组聚合 |
| JOIN | 多表连接（INNER/LEFT/RIGHT/FULL OUTER） |
| 跨 Schema | 使用 `schema.table` 格式引用其他 schema 的表 |

---

### 7. 通用类型

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

### 8. 全文索引

#### CreateIndexRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| index_name | string | 是 | 索引名称 |
| fields | repeated IndexFieldDef | 是 | 字段定义列表 |

#### IndexFieldDef

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| name | string | 是 | 字段名称 |
| field_type | int32 | 是 | 字段类型（0=STRING, 1=INT64, 2=BYTES, 3=FLOAT64） |
| comment | string | 否 | 字段注释 |

#### CreateIndexResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| index_id | uint64 | 索引 ID（Snowflake ID） |
| message | string | 错误信息 |

#### DropIndexRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| index_name | string | 是 | 索引名称 |

#### DropIndexResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| message | string | 错误信息 |

#### ListIndicesRequest

无参数

#### ListIndicesResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| index_names | repeated string | 索引名称列表 |
| message | string | 错误信息 |

#### GetIndexFieldsRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| index_name | string | 是 | 索引名称 |

#### GetIndexFieldsResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| fields | repeated ColumnMeta | 字段定义列表 |
| message | string | 错误信息 |

#### GetIndexMetaRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| index_name | string | 是 | 索引名称 |

#### GetIndexMetaResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| index_id | uint64 | 索引 ID |
| index_name | string | 索引名称 |
| column_count | uint32 | 字段数量 |
| comment | string | 索引注释 |
| message | string | 错误信息 |

#### GetIndexStatsRequest

无参数

#### GetIndexStatsResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| total_indices | uint32 | 索引总数 |
| index_names | repeated string | 索引名称列表 |
| message | string | 错误信息 |

#### AddDocumentRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| index_name | string | 是 | 索引名称 |
| doc_id | string | 否 | 文档 ID（不提供则自动生成 Snowflake ID） |
| fields | map<string, string> | 是 | 文档字段键值对 |

#### AddDocumentResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| doc_id | string | 文档 ID（用户提供或自动生成） |
| message | string | 错误信息 |

#### GetDocumentRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| index_name | string | 是 | 索引名称 |
| doc_id | string | 是 | 文档 ID |

#### GetDocumentResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| doc_id | string | 文档 ID |
| fields | map<string, string> | 文档字段 |
| message | string | 错误信息 |

#### DeleteDocumentRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| index_name | string | 是 | 索引名称 |
| doc_id | string | 是 | 文档 ID |

#### DeleteDocumentResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| message | string | 错误信息 |

#### SearchIndexRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| index_name | string | 是 | 索引名称 |
| query | string | 是 | 搜索查询字符串 |
| limit | uint32 | 否 | 返回结果数量限制（默认 10） |
| field_queries | map<string, string> | 否 | 多字段搜索（字段名 -> 查询值） |

#### SearchIndexResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| results | repeated SearchResultItem | 搜索结果列表 |
| message | string | 错误信息 |

#### SearchResultItem

| 字段 | 类型 | 说明 |
|------|------|------|
| doc_id | string | 文档 ID |
| score | float | 匹配分数 |
| fields | map<string, string> | 文档字段 |

---

### 9. 向量化服务

#### EmbeddingRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| model_name | string | 是 | 模型名称（需先注册） |
| texts | repeated string | 是 | 要生成向量的文本列表 |
| dim | int32 | 是 | 向量维度 |

#### EmbeddingResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| message | string | 提示信息 |
| results | repeated EmbeddingResult | 向量化结果列表 |

#### EmbeddingResult

| 字段 | 类型 | 说明 |
|------|------|------|
| text | string | 原始文本 |
| embedding | repeated float | 生成的向量 |
| dim | int32 | 向量维度 |

#### SimilarityRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| model_name | string | 是 | 模型名称 |
| query_embedding | repeated float | 是 | 查询向量 |
| candidates | repeated EmbeddingResult | 是 | 候选向量列表 |
| top_k | int32 | 是 | 返回最相似的 top_k 个结果 |

#### SimilarityResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| message | string | 提示信息 |
| results | repeated SimilarityResult | 相似度结果列表 |

#### SimilarityResult

| 字段 | 类型 | 说明 |
|------|------|------|
| text | string | 文本内容 |
| embedding | repeated float | 向量 |
| score | float | 相似度分数 |
| rank | int32 | 排名（1 为最相似） |

#### ModelInfoRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| model_name | string | 是 | 模型名称 |

#### ModelInfoResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| message | string | 提示信息 |
| model_name | string | 模型名称 |
| embedding_dim | int32 | 向量维度 |
| model_path | string | 模型路径 |
| device | string | 运行设备（Cpu / Cuda） |
| loaded | bool | 是否已加载 |

#### ListModelsRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| (无) | - | - | 无参数 |

#### ListModelsResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| message | string | 提示信息 |
| models | repeated ModelInfoResponse | 已注册的模型列表 |

#### LoadModelRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| model_name | string | 是 | 模型名称 |
| model_path | string | 是 | 模型路径 |
| embedding_dim | int32 | 是 | 向量维度 |

#### LoadModelResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| message | string | 提示信息 |
| model_name | string | 模型名称 |

#### UnloadModelRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| model_name | string | 是 | 模型名称 |

#### UnloadModelResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| message | string | 提示信息 |
| model_name | string | 模型名称 |

#### ListLoadableModelsRequest

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| (无) | - | - | 无参数 |

#### LoadableModelInfo

| 字段 | 类型 | 说明 |
|------|------|------|
| model_name | string | 模型目录名称 |
| model_path | string | 模型完整路径 |
| embedding_dim | int32 | 向量维度（来自 config.json 的 hidden_size） |
| has_config | bool | 是否存在 config.json |
| has_tokenizer | bool | 是否存在 tokenizer.json |
| has_weights | bool | 是否存在 model.safetensors |
| is_loaded | bool | 是否已加载到内存 |

#### ListLoadableModelsResponse

| 字段 | 类型 | 说明 |
|------|------|------|
| success | bool | 操作是否成功 |
| message | string | 提示信息 |
| model_dir | string | Candle 模型目录路径 |
| models | repeated LoadableModelInfo | 可加载模型列表 |

#### 自动加载配置

启动时通过 `laoflchdb.yaml` 的 `vector_service` 配置节控制模型自动加载：

```yaml
vector_service:
  enabled: true               # 启用向量化服务
  auto_load: true             # 启动时自动扫描加载
  load_models: []             # 指定加载列表（空=加载 candle 目录下所有有效模型）
```

模型文件需放置于 `{model_path}/candle/{model_name}/` 目录下，包含：
- `config.json` — BERT 模型配置
- `tokenizer.json` — HuggingFace tokenizer
- `model.safetensors` — 模型权重 (SafeTensors 格式)

---

## 使用示例

### Python 示例

```python
import grpc
import rpc_pb2
import rpc_pb2_grpc
import vector_pb2
import vector_pb2_grpc

# 建立连接
channel = grpc.insecure_channel("localhost:19777")
stub = rpc_pb2_grpc.LaoflchDbStub(channel)
vec_stub = vector_pb2_grpc.VectorServiceStub(channel)

# 1. 用户登录（获取 Token）
login_resp = stub.Login(rpc_pb2.LoginRequest(
    username="admin",
    password="laoflchdb"
))
print(f"Login: {login_resp.success}, token={login_resp.token[:20]}...")

# 创建认证元数据
metadata = [('authorization', f'Bearer {login_resp.token}')]

# 2. 获取版本信息
ver_resp = stub.GetVersion(rpc_pb2.GetVersionRequest())
print(f"Version: {ver_resp.version}")

# ===== KV 操作 =====
# 3. 创建表
resp = stub.CreateTable(rpc_pb2.CreateTableRequest(
    schema="sys",
    table_name="users",
    columns=[
        rpc_pb2.ColumnDef(name="id", column_type=1),
        rpc_pb2.ColumnDef(name="name", column_type=2),
        rpc_pb2.ColumnDef(name="email", column_type=2),
    ]
), metadata=metadata)
print(f"Create table: {resp.success}")

# 4. 插入数据
resp = stub.Put(rpc_pb2.PutRequest(
    schema="sys",
    table="users",
    key=b"user_001",
    value=b'{"id":1,"name":"Alice","email":"alice@example.com"}'
), metadata=metadata)
print(f"Put: {resp.success}")

# 5. 读取数据
resp = stub.Get(rpc_pb2.GetRequest(
    schema="sys", table="users", key=b"user_001"
), metadata=metadata)
print(f"Get: {resp.success}, value={resp.value.decode()}")

# 6. 查询数据
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
                            value=rpc_pb2.Field(
                                integer_value=rpc_pb2.IntegerValue(value=0)
                            )
                        )
                    ]
                )
            ]
        )
    ],
    limit=10
), metadata=metadata)
print(f"Query: {resp.success}, rows={len(resp.rows)}")

# 7. SQL 查询
resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(
    schema="sys",
    sql="SELECT * FROM users WHERE id > 0 LIMIT 5"
), metadata=metadata)
print(f"SQL Query: {resp.success}, rows={len(resp.rows)}")

# 8. 跨 Schema JOIN 查询
resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(
    schema="sales",
    sql="""
        SELECT sales.orders.order_id, inventory.products.product_name 
        FROM sales.orders 
        JOIN inventory.products ON sales.orders.product_id = inventory.products.product_id
        LIMIT 10
    """
), metadata=metadata)
print(f"Cross-schema JOIN: {resp.success}, rows={len(resp.rows)}")

# ===== 全文索引操作 =====
# 9. 创建索引
resp = stub.CreateIndex(rpc_pb2.CreateIndexRequest(
    index_name="my_index",
    fields=[
        rpc_pb2.IndexFieldDef(name="title", field_type=0),
        rpc_pb2.IndexFieldDef(name="content", field_type=0),
    ]
), metadata=metadata)
print(f"Create index: {resp.success}, id={resp.index_id}")

# 10. 添加文档
resp = stub.AddDocument(rpc_pb2.AddDocumentRequest(
    index_name="my_index",
    doc_id="doc_001",
    fields={"title": "Hello", "content": "World"}
), metadata=metadata)
print(f"Add document: {resp.success}")

# 11. 搜索索引
resp = stub.SearchIndex(rpc_pb2.SearchIndexRequest(
    index_name="my_index",
    query="Hello",
    limit=10
), metadata=metadata)
print(f"Search: {resp.success}, hits={len(resp.results)}")

# ===== 向量化服务操作 =====
# 12. 注册模型
resp = vec_stub.LoadModel(vector_pb2.LoadModelRequest(
    model_name="bert_base",
    model_path="/tmp/models/bert_base",
    embedding_dim=768,
), metadata=metadata)
print(f"Load model: {resp.success}")

# 13. 生成向量
resp = vec_stub.CreateEmbedding(vector_pb2.EmbeddingRequest(
    model_name="bert_base",
    texts=["Hello World", "Rust Programming"],
    dim=768,
), metadata=metadata)
for r in resp.results:
    print(f"  text='{r.text[:20]}' embedding[:3]={r.embedding[:3]}")

# 14. 计算相似度
candidates = [
    vector_pb2.EmbeddingResult(text="Rust", embedding=[1.0, 0.0, 0.0], dim=3),
    vector_pb2.EmbeddingResult(text="Python", embedding=[0.9, 0.1, 0.0], dim=3),
]
resp = vec_stub.ComputeSimilarity(vector_pb2.SimilarityRequest(
    model_name="test",
    query_embedding=[1.0, 0.0, 0.0],
    candidates=candidates,
    top_k=2,
), metadata=metadata)
for r in resp.results:
    print(f"  rank={r.rank}: '{r.text}' score={r.score:.4f}")

# 15. 列出模型
resp = vec_stub.ListModels(vector_pb2.ListModelsRequest(), metadata=metadata)
for m in resp.models:
    print(f"  model: {m.model_name}, dim={m.embedding_dim}, device={m.device}")

# 16. 列出可加载模型（扫描 candle 目录）
resp = vec_stub.ListLoadableModels(vector_pb2.ListLoadableModelsRequest(), metadata=metadata)
print(f"ListLoadableModels: {resp.success}, dir={resp.model_dir}")
for m in resp.models:
    status = "LOADED" if m.is_loaded else "available"
    print(f"  {m.model_name}: dim={m.embedding_dim} [{status}]")

# 17. 删除表
resp = stub.DropTable(rpc_pb2.DropTableRequest(
    schema="sys", table_name="users"
), metadata=metadata)
print(f"Drop table: {resp.success}")

# 18. 用户登出
resp = stub.Logout(rpc_pb2.LogoutRequest(token=login_resp.token))
print(f"Logout: {resp.success}")
```

### Go 示例

```go
package main

import (
    "context"
    "fmt"
    "log"

    "google.golang.org/grpc"
    "google.golang.org/grpc/metadata"
    pb "path/to/proto"
)

func main() {
    conn, err := grpc.Dial("localhost:19777", grpc.WithInsecure())
    if err != nil {
        log.Fatalf("did not connect: %v", err)
    }
    defer conn.Close()
    
    client := pb.NewLaoflchDbClient(conn)

    // 1. 用户登录
    loginResp, err := client.Login(context.Background(), &pb.LoginRequest{
        Username: "admin",
        Password: "laoflchdb",
    })
    if err != nil {
        log.Fatalf("Login failed: %v", err)
    }
    fmt.Printf("Login: %v, Token: %s\n", loginResp.Success, loginResp.Token)

    // 创建认证上下文
    ctx := metadata.AppendToOutgoingContext(context.Background(), 
        "authorization", "Bearer "+loginResp.Token)

    // 2. 创建表
    resp, err := client.CreateTable(ctx, &pb.CreateTableRequest{
        Schema:    "sys",
        TableName: "users",
        Columns: []*pb.ColumnDef{
            {Name: "id", ColumnType: 1},
            {Name: "name", ColumnType: 2},
        },
    })
    fmt.Printf("Create table: %v\n", resp.Success)

    // 3. 用户登出
    logoutResp, err := client.Logout(context.Background(), &pb.LogoutRequest{
        Token: loginResp.Token,
    })
    fmt.Printf("Logout: %v\n", logoutResp.Success)
}
```

---

## 自动测试

```bash
# 运行索引服务 gRPC 测试
python3 tests_python/test_index_grpc.py

# 运行向量化服务 gRPC 测试
python3 tests_python/test_vector_service_grpc.py

# 运行完整测试
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
| UNAUTHENTICATED (16) | 未认证（无效或缺失的 Token） |
| NOT_FOUND (5) | 资源不存在（如模型未找到） |