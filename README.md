# laoflchDB-rust: Rust + RocksDB 封装 OLTP 数据库

基于 Rust + RocksDB 的单机 OLTP 数据库，支持 gRPC 和 REST API 接口，命令行启动独立运行。

---

## 编译环境要求

- GCC >= 13.2.0 (rust-rocksdb 0.50 C++20 兼容)
- Rust 1.75+

---

## 总体架构设计

laoflchDB 采用 **模块化分层架构设计**，核心实体为 `LaoflchDBServer`：

| 层次模块 | 位置 | 说明 |
|----------|------|------|
| **Server 层** | [src/server/mod.rs](src/server/mod.rs) | LaoflchDBServer 总入口，支持多协议启动 |
| **Access 层** | [src/access/mod.rs](src/access/mod.rs) | 接入服务层，负责协议接入和路由 |
| **Service 层** | [src/service/mod.rs](src/service/mod.rs) | 数据库基础服务能力 + SchemaManager |
| **SQL 引擎** | [laoflchdb_sql_df_engine/src/lib.rs](laoflchdb_sql_df_engine/src/lib.rs) | DataFusion SQL 解析和执行引擎 |
| **DBEngine 接口** | [laoflchdb_engines/](laoflchdb_engines/) | 独立的数据库引擎接口定义 crate |
| **RocksDB 引擎** | [multi_table_rocksdb/](multi_table_rocksdb/) | 独立的 RocksDB 引擎实现 crate |
| CLI 命令行 | [src/cli/mod.rs](src/cli/mod.rs) | clap 命令行参数解析 |
| 配置模块 | [src/config/](src/config/) | YAML 配置文件解析 |
| lib.rs | [src/lib.rs](src/lib.rs) | 库导出入口 |
| main.rs | [src/main.rs](src/main.rs) | 二进制 standalone 程序入口 |

**目录结构源码树：**
```
laoflchDB-rust/
├── src/
│   ├── access/          # 接入服务层: AccessService, GrpcService, RestService
│   │   ├── rest.rs      # REST API 服务 (Axum)
│   │   ├── proto/      # gRPC RPC 服务接口定义
│   │   │   └── rpc.proto
│   │   └── mod.rs
│   ├── server/          # Server 层: LaoflchDBServer 总入口
│   │   └── mod.rs
│   ├── service/         # Service 层: DatabaseService + SchemaManager
│   │   └── mod.rs
│   ├── cli/             # 命令行 Parser: start / init
│   │   └── mod.rs
│   ├── config/          # YAML 配置解析
│   │   └── mod.rs
│   ├── lib.rs           # 库模块 + proto 导出
│   └── main.rs          # 二进制入口
├── laoflchdb_engines/   # 独立的 DBEngine 接口定义 crate
│   ├── proto/
│   │   ├── field.proto
│   │   ├── metadata.proto
│   │   ├── query.proto
│   │   └── row.proto
│   ├── src/
│   │   ├── field.rs
│   │   ├── lib.rs
│   │   ├── metadata.rs
│   │   ├── mod.rs
│   │   ├── query.rs
│   │   └── row.rs
│   └── Cargo.toml
├── laoflchdb_sql_df_engine/  # SQL 引擎 crate - DataFusion 集成
│   ├── src/lib.rs
│   ├── Cargo.toml
│   └── README.md
├── laoflchdb_index_tantivy_engine/  # 全文索引引擎 crate - Tantivy
│   ├── src/lib.rs
│   └── Cargo.toml
├── laoflchdb_vector_service/  # 向量化服务 crate - Candle/BERT/视觉模型
│   ├── proto/
│   │   └── vector.proto
│   ├── src/
│   │   ├── lib.rs
│   │   └── vision_encoder.rs    # ViT 视觉编码器
│   ├── Cargo.toml
│   └── build.rs
├── laoflchdb_embedding_service/  # 嵌入向量索引服务 crate - HNSW
│   ├── proto/
│   │   └── embedding.proto
│   ├── src/lib.rs
│   ├── Cargo.toml
│   └── build.rs
├── laoflchdb_object_store_service/  # 对象存储服务 crate - S3 兼容（BlobDB）
│   ├── proto/
│   │   └── object_store.proto
│   ├── src/lib.rs
│   ├── Cargo.toml
│   └── build.rs
├── laoflchdb_kv_rocksdb_engine/  # 独立的 KV 引擎 crate
│   ├── src/lib.rs
│   └── Cargo.toml
├── multi_table_rocksdb/  # 独立的 RocksDB 引擎实现 crate
│   ├── src/
│   │   ├── lib.rs
│   │   ├── multi_table_rocksdb.rs
│   │   └── rocksdb_table.rs    # RocksDBTable + RocksScanExec 物理算子
│   └── Cargo.toml
├── tests/               # Rust 单元测试和集成测试
│   ├── basic_uuid_tests.rs
│   ├── protobuf_tests.rs
│   ├── rest_tests.rs
│   ├── permission_tests.rs
│   ├── prefix_filter_tests.rs
│   ├── integration_tests.rs
│   ├── index_service_tests.rs
│   ├── index_tantivy_integration_tests.rs
│   └── cross_schema_join_tests.rs
├── tests_python/        # Python 自动化 E2E 测试
│   ├── test_e2e_grpc.py
│   ├── test_e2e_rest.py
│   ├── test_sql_query_validation.py    # SQL 查询验证测试
│   ├── test_grpc_sql_query.py          # gRPC SQL 查询测试
│   ├── test_vector_service_grpc.py     # 向量化服务测试
│   ├── test_embedding_service_grpc.py  # 嵌入向量索引服务测试
│   ├── test_index_grpc.py              # 全文索引 gRPC 测试
│   ├── test_object_store_service_grpc.py # 对象存储服务 gRPC 测试
│   ├── test_object_store_service_rest.py # 对象存储服务 REST 测试（S3 兼容性）
│   └── test_final.py
├── xtask/              # 构建和测试任务
│   ├── src/main.rs
│   └── Cargo.toml
├── run_tests.sh        # Rust 测试脚本
├── run_all_tests.sh    # 完整测试套件 (Rust + Python)
├── verify_tests.sh     # 快速验证测试
├── laoflchdb.yaml      # 配置文件示例
├── README.md
├── design.md           # 详细设计文档
├── TESTING.md          # 测试指南
├── TEST_COVERAGE.md    # 测试覆盖报告
└── Cargo.toml
```

---

## 新增：跨 Schema JOIN 支持

### 核心特性

| 特性 | 说明 |
|------|------|
| **跨 Schema 查询** | 支持在不同 Schema 之间执行 JOIN 操作 |
| **多表 JOIN** | 支持三表及以上的跨 Schema JOIN |
| **JOIN 类型** | 支持 INNER JOIN、LEFT JOIN |
| **WHERE 条件** | 支持在跨 Schema JOIN 中使用 WHERE 过滤 |
| **聚合函数** | 支持跨 Schema JOIN 后的聚合操作 |

### 跨 Schema JOIN 示例

```sql
-- 跨 Schema INNER JOIN
SELECT sales.orders.order_id, inventory.products.product_name 
FROM sales.orders 
JOIN inventory.products ON sales.orders.product_id = inventory.products.product_id;

-- 跨 Schema LEFT JOIN
SELECT sales.orders.order_id, inventory.products.product_name 
FROM sales.orders 
LEFT JOIN inventory.products ON sales.orders.product_id = inventory.products.product_id;

-- 三表跨 Schema JOIN
SELECT sales.customers.customer_name, inventory.products.product_name, sales.orders.order_id 
FROM sales.customers 
JOIN sales.orders ON sales.customers.customer_id = sales.orders.customer_id 
JOIN inventory.products ON sales.orders.product_id = inventory.products.product_id;

-- 跨 Schema JOIN 带 WHERE 条件
SELECT sales.orders.order_id, inventory.products.product_name 
FROM sales.orders 
JOIN inventory.products ON sales.orders.product_id = inventory.products.product_id 
WHERE inventory.products.category = 'Electronics';
```

### Schema 命名约定

- **默认 Schema**: `sys`（不带 Schema 前缀的表名默认使用 `sys` Schema）
- **跨 Schema 引用**: 使用 `schema_name.table_name` 格式引用其他 Schema 的表

---

## 新增：SQL 查询优化（Filter/Project/Limit 下推）

### 核心特性

| 优化类型 | 说明 | 支持的操作符 |
|---------|------|-------------|
| **Filter 下推** | 将谓词过滤下推到存储层 | `=`, `!=`, `<`, `>`, `<=`, `>=`, `AND`, `OR` |
| **Project 下推** | 只扫描需要的列 | 任意列组合 |
| **Limit 下推** | 提前终止扫描 | `LIMIT n` |

### 自定义物理执行算子

`RocksScanExec` 替代了 DataFusion 默认的 MemTable，直接对接 RocksDB：

```
SQL 查询 → DataFusion 优化器 → TableProvider.scan()
    → RocksScanExec (自定义物理算子)
    → table_to_arrow_with_pushdown()
    → 存储引擎直接扫描
```

### 逻辑表达式下推支持

- **AND 条件**：不同列之间的条件为 AND 关系
- **OR 条件**：同一列的多个条件为 OR 关系
- **组合表达式**：支持 `(age > 25 AND age < 40) OR score > 92`

### SQL 查询示例

```bash
# 谓词下推
SELECT name, age FROM users WHERE age > 30

# 列投影下推（只扫描 name 和 age 列）
SELECT name, age FROM users

# Limit 下推（最多返回 10 条）
SELECT * FROM users LIMIT 10

# 组合条件
SELECT * FROM users WHERE (age > 25 AND score > 90) OR name = 'Alice'
```

---

## 新增：全文索引引擎 (Tantivy)

### 核心特性

| 特性 | 说明 |
|------|------|
| **全文搜索** | 基于 Tantivy 0.26 实现全文索引 |
| **Schema 集成** | 自动为每个表创建对应的全文索引 |
| **多字段搜索** | 支持在多个字段上进行搜索 |
| **Snowflake ID** | 自动生成分布式唯一 ID |
| **并发安全** | 使用 `RwLock` 和 `Mutex` 确保线程安全 |

### 全文索引 API

| 操作 | gRPC | REST |
|------|------|------|
| 创建索引 | `CreateIndex` | `POST /api/v1/index/indices` |
| 删除索引 | `DropIndex` | `DELETE /api/v1/index/indices/{name}` |
| 列出索引 | `ListIndices` | `GET /api/v1/index/indices` |
| 添加文档 | `AddDocument` | `POST /api/v1/index/indices/{name}/docs` |
| 获取文档 | `GetDocument` | `GET /api/v1/index/indices/{name}/docs/{doc_id}` |
| 删除文档 | `DeleteDocument` | `DELETE /api/v1/index/indices/{name}/docs/{doc_id}` |
| 搜索 | `SearchIndex` | `POST /api/v1/index/indices/{name}/search` |

### 索引配置示例

```json
{
  "name": "my_index",
  "fields": [
    {"name": "title", "type": "TEXT", "indexed": true, "stored": true},
    {"name": "content", "type": "TEXT", "indexed": true, "stored": true},
    {"name": "category", "type": "STRING", "indexed": true, "stored": true},
    {"name": "view_count", "type": "INT", "indexed": true, "stored": true}
  ]
}
```

---

## 新增：向量化服务 (VectorService)

### 核心特性

| 特性 | 说明 |
|------|------|
| **文本向量化** | 基于 Candle 0.10 + CUDA 实现 BERT/XLM-RoBERTa 模型推理 |
| **图片向量化** | 支持 ViT 架构视觉模型 (Jina-CLIP-v2, SigLIP2) |
| **模型自动加载** | 启动时通过配置自动加载指定模型 |
| **L2 归一化** | 输出向量自动进行 L2 归一化 |
| **gRPC API** | 提供完整的 gRPC 向量化服务接口 |

### 支持的模型

| 模型 | 架构 | 类型 | 维度 |
|------|------|------|------|
| bge-small-zh-v1.5 | BERT | 文本 | 512 |
| bge-m3 | XLM-RoBERTa | 文本 | 1024 |
| jina-clip-v2 | ViT-L/14 | 视觉 | 1024 |
| siglip2 | ViT-B/16 | 视觉 | 768 |

### 图片向量化流程

```
图片输入 (PNG/JPEG) → ImageProcessor 解码
    → resize 到模型指定尺寸
    → 归一化 (mean/std)
    → 转 Tensor
    → Patch Embedding (Conv2d)
    → 添加 CLS Token + Position Embedding
    → Transformer Encoder 推理
    → CLS 向量输出
    → L2 归一化
```

### 向量化 API

| 操作 | gRPC | 说明 |
|------|------|------|
| 创建向量 | `CreateEmbedding` | 文本/图片 → 向量 |
| 计算相似度 | `ComputeSimilarity` | 向量间余弦相似度 |
| 加载模型 | `LoadModel` | 加载指定模型 |
| 卸载模型 | `UnloadModel` | 卸载指定模型 |
| 列出模型 | `ListModels` | 列出已加载模型 |
| 可加载模型 | `ListLoadableModels` | 列出可加载的模型列表 |
| 模型信息 | `GetModelInfo` | 获取模型详细信息 |

### 配置示例

```yaml
vector_service:
  enabled: true
  auto_load: true
  load_models: ["bge-small-zh-v1.5", "bge-m3", "jina-clip-v2", "siglip2"]
```

---

## 新增：嵌入向量索引服务 (EmbeddingIndexService)

### 核心特性

| 特性 | 说明 |
|------|------|
| **ANN 搜索** | 基于 HNSW 算法实现近似最近邻搜索 |
| **向量持久化** | 基于 RocksDB 的向量数据持久化存储 |
| **快照管理** | 支持 HNSW 图拓扑快照保存和加载 |
| **批量插入** | 支持批量向量插入 |
| **范围搜索** | 支持范围搜索 |

### 配置参数

| 参数 | 说明 | 默认值 |
|------|------|--------|
| dim | 向量维度 | 512 |
| m | HNSW 图每个节点的最大连接数 | 32 |
| ef_construction | 图构建时的搜索宽度 | 200 |
| ef_search | 搜索时的搜索宽度 | 50 |
| max_elements | 索引最大容量 | 1000000 |

### 嵌入向量 API

| 操作 | gRPC | 说明 |
|------|------|------|
| 插入向量 | `InsertEmbedding` | 插入向量到索引 |
| 搜索向量 | `SearchEmbedding` | 搜索 Top-K 最近邻 |
| 删除向量 | `DeleteEmbedding` | 从索引中删除向量 |
| 获取信息 | `GetIndexInfo` | 获取索引统计信息 |
| 保存快照 | `SaveSnapshot` | 保存索引快照 |
| 加载快照 | `LoadSnapshot` | 加载索引快照 |

### 配置示例

```yaml
embedding_index:
  enabled: true
  dim: 512
  m: 32
  ef_construction: 200
  ef_search: 50
  max_elements: 1000000
  kv_db_path: ./laoflch_hnsw_data
  snapshot_path: ./laoflch_hnsw_snapshots
```

---

## 新增：对象存储服务 (ObjectStoreService) - S3 兼容

### 核心特性

| 特性 | 说明 |
|------|------|
| **S3 兼容 API** | 提供 S3 风格的 Bucket/Object REST API 和完整 gRPC API |
| **大对象存储** | 基于 RocksDB BlobDB 实现大对象分离存储 |
| **元数据管理** | 自动维护 content_type、etag、last_modified、用户元数据 |
| **目录结构模拟** | 通过 delimiter + prefix 支持 S3 风格的目录层级 |
| **幂等操作** | Bucket 创建/删除、对象删除均为幂等操作 |
| **跨 Bucket 复制** | 支持 CopyObject 在不同 Bucket 间复制对象 |
| **批量删除** | 支持 DeleteObjects 批量删除多个对象 |

### 存储模型

每个 Bucket 对应一个 RocksDB 表（Column Family），对象数据通过 BlobDB 存储：

```
Bucket (RocksDB Table)
├── __obj__{key}      → 对象二进制数据（存储在 BlobDB）
├── __meta__{key}     → 对象元数据 JSON
│   {
│     "key": "photos/cat.jpg",
│     "content_type": "image/jpeg",
│     "content_length": 102400,
│     "etag": "\"a1b2c3d4...\"",
│     "last_modified": "1720000000",
│     "user_metadata": {...}
│   }
└── __bucket_meta__   → Bucket 元数据（创建时间）
```

### 对象存储 API

#### gRPC API

| 操作 | gRPC RPC | 说明 |
|------|---------|------|
| 创建 Bucket | `CreateBucket` | 创建存储桶（幂等） |
| 删除 Bucket | `DeleteBucket` | 删除存储桶 |
| 列出 Buckets | `ListBuckets` | 列出所有存储桶 |
| 上传对象 | `PutObject` | 上传对象数据 |
| 下载对象 | `GetObject` | 下载对象数据 |
| 删除对象 | `DeleteObject` | 删除单个对象（幂等） |
| 批量删除 | `DeleteObjects` | 批量删除多个对象 |
| 列出对象 | `ListObjects` | 列出 Bucket 中的对象 |
| 获取元数据 | `HeadObject` | 仅获取对象元数据 |
| 复制对象 | `CopyObject` | 跨 Bucket 复制对象 |

#### REST API（S3 兼容）

所有 REST 端点挂载在 `/api/v1/object-store` 前缀下：

| 操作 | HTTP 方法 & 路径 | 说明 |
|------|------------------|------|
| ListBuckets | `GET /api/v1/object-store` | 列出所有 Bucket |
| CreateBucket | `PUT /api/v1/object-store/{bucket}` | 创建 Bucket |
| DeleteBucket | `DELETE /api/v1/object-store/{bucket}` | 删除 Bucket |
| ListObjects | `GET /api/v1/object-store/{bucket}` | 列出对象（支持 prefix/delimiter/max_keys/marker） |
| PutObject | `PUT /api/v1/object-store/{bucket}/{key}` | 上传对象 |
| GetObject | `GET /api/v1/object-store/{bucket}/{key}` | 下载对象 |
| HeadObject | `HEAD /api/v1/object-store/{bucket}/{key}` | 获取对象元数据 |
| DeleteObject | `DELETE /api/v1/object-store/{bucket}/{key}` | 删除对象 |

### 配置示例

```yaml
object_store:
  enabled: true                        # 启用对象存储服务
  db_path: ./laoflch_object_store_data # BlobDB 数据目录
  schema_name: object_store            # Schema 名称
  blob_db:
    enabled: true                      # 启用 BlobDB
    min_blob_size: 0                   # 最小大对象阈值（字节）
    blob_file_size: 268435456          # Blob 文件大小（默认 256MB）
    blob_compression_type: zstd        # 压缩算法（zstd/lz4/snappy/none）
    enable_blob_garbage_collection: true
    blob_garbage_collection_age_cutoff: 0.25
```

### REST API 使用示例

```bash
# 列出所有 Bucket
curl http://localhost:8080/api/v1/object-store \
  -H "Authorization: Bearer <your_token>"

# 创建 Bucket
curl -X PUT http://localhost:8080/api/v1/object-store/my-bucket \
  -H "Authorization: Bearer <your_token>"

# 上传文件
curl -X PUT http://localhost:8080/api/v1/object-store/my-bucket/photos/cat.jpg \
  -H "Content-Type: image/jpeg" \
  -H "Authorization: Bearer <your_token>" \
  --data-binary @/path/to/cat.jpg

# 下载文件
curl http://localhost:8080/api/v1/object-store/my-bucket/photos/cat.jpg \
  -H "Authorization: Bearer <your_token>" \
  -o cat.jpg

# 列出对象（带前缀和分隔符）
curl "http://localhost:8080/api/v1/object-store/my-bucket?prefix=photos/&delimiter=/" \
  -H "Authorization: Bearer <your_token>"

# 删除对象
curl -X DELETE http://localhost:8080/api/v1/object-store/my-bucket/photos/cat.jpg \
  -H "Authorization: Bearer <your_token>"
```

**位置**: [laoflchdb_object_store_service/src/lib.rs](laoflchdb_object_store_service/src/lib.rs)

---

## 新增：优雅关闭功能

### 核心特性

| 特性 | 说明 |
|------|------|
| **信号处理** | 支持 SIGINT (Ctrl+C)、SIGTERM (kill 命令) 信号 |
| **RocksDB 刷新** | 关闭时自动调用 `db.flush()` 确保数据持久化 |
| **锁释放** | 正确释放 RocksDB 锁文件，避免下次启动报错 |
| **Schema 遍历** | 遍历所有 Schema 并优雅关闭对应的引擎 |

### 关闭流程

```
收到信号 (Ctrl+C/SIGTERM)
    ↓
调用 service.shutdown()
    ↓
遍历所有 Schema
    ↓
对每个 Schema 调用 engine.shutdown()
    ↓
执行 db.flush() 刷新数据到磁盘
    ↓
释放锁文件
    ↓
服务正常退出
```

### 关闭方式

```bash
# 方式 1: Ctrl+C (终端中)
^C

# 方式 2: kill 命令
kill <pid>

# 方式 3: kill -TERM
kill -TERM <pid>
```

---

## 1. Schema 与 RocksDB 映射设计

### 核心映射关系

**每个 Schema 是一个独立的 RocksDB 实例**：

```
db_path/ (根目录)
├── sys/              # sys Schema (RocksDB 实例) - 默认初始化
│   ├── default       # default CF: 存储 db/table/col 元数据
│   └── user          # user 表 CF: 存储 user 表数据
│
└── analytics/        # analytics Schema (RocksDB 实例)
    ├── default       # default CF: 存储 analytics 的元数据
    └── events        # events 表 CF
```

### Column Family 设计

| Column Family | 用途 |
|---------------|------|
| `default` | 存储 db、table、col 级别的元数据 |
| `{table_name}` | 存储对应表的数据 |

### 元数据 Key 格式

| 类型 | Key 格式 | 示例 |
|------|----------|------|
| Schema 元数据 | `META-SCHEMA:{schema_name}` | `META-SCHEMA:sys` |
| 表元数据 | `META-TABLE:{table_name}:{table_id}` | `META-TABLE:user:0` |
| 字段元数据 | `META-COL:{table_id_fixed}:{column_name}:{column_id}:{column_type}` | `META-COL:00000000000000000000:user_id:0:COLUMN_TYPE_INT64` |

**注意**: 其中 `{table_id_fixed}` 是固定长度 20 字符的字符串，不足时前面补0。

### 自增 ID 设计

| ID 类型 | 起始值 | 存储位置 | 说明 |
|---------|--------|----------|------|
| `table_id` | 0 | `SchemaMeta.next_auto_inc_table_id` | Schema 内唯一，每创建表 +1 |
| `column_id` | 0 | `TableMeta.next_auto_inc_column_id` | 表内唯一，每添加字段 +1 |

### MAX_TABLE_ID_LENGTH 常量

在 `multi_table_rocksdb` crate 中定义，用于固定长度的 table_id 字符串格式：
```rust
pub const MAX_TABLE_ID_LENGTH: usize = 20;
```

---

## 2. LaoflchDBServer 总入口架构

`LaoflchDBServer` 是整个数据库的统一入口实体，负责组装和启动服务：

```
┌─────────────────────────────────────────────────────────────┐
│                     LaoflchDBServer                          │
│                                                             │
│  ┌──────────────────────────────────────────────────────┐  │
│  │                  Access 层 (接入层)                    │  │
│  │  ┌─────────────┐ ┌─────────────┐                     │  │
│  │  │ GrpcService  │ │ RestService  │                     │  │
│  │  │   (gRPC)     │ │   (HTTP)     │                     │  │
│  │  └─────────────┘ └─────────────┘                     │  │
│  │                        ↑                            │  │
│  │              AccessService (统一接口)                 │  │
│  └───────────────────────┼──────────────────────────────┘  │
│                          ↓                                   │
│  ┌──────────────────────────────────────────────────────┐  │
│  │                  Service 层 (服务层)                   │  │
│  │  ┌─────────────────────────────────────────────────┐ │  │
│  │  │              SchemaManager                       │ │  │
│  │  │  ┌─────────────┐ ┌─────────────┐ ┌───────────┐ │ │  │
│  │  │  │DatabaseService│ │DatabaseService│ │Database...│ │ │  │
│  │  │  │   (sys)      │ │(analytics)   │ │(warehouse)│ │ │  │
│  │  │  └─────────────┘ └─────────────┘ └───────────┘ │ │  │
│  │  └─────────────────────────────────────────────────┘ │  │
│  └───────────────────────┬──────────────────────────────┘  │
│                          ↓                                   │
│  ┌──────────────────────────────────────────────────────┐  │
│  │              SQLEngine 层 (SQL 引擎)                   │  │
│  │  ┌─────────────────────────────────────────────────┐ │  │
│  │  │          DataFusionSQLEngine                     │ │  │
│  │  │  - SQL 解析 (DataFusion)                         │ │  │
│  │  │  - 查询规划与优化                                 │ │  │
│  │  │  - Arrow 列式数据转换                            │ │  │
│  │  │  - 查询下推优化 (Filter/Project/Limit)           │ │  │
│  │  └─────────────────────────────────────────────────┘ │  │
│  └───────────────────────┬──────────────────────────────┘  │
│                          ↓                                   │
│  ┌──────────────────────────────────────────────────────┐  │
│  │                  DBEngine 层 (引擎层)                  │  │
│  │  ┌─────────────────┐ ┌─────────────────┐            │  │
│  │  │MultiTableRocksDB│ │MultiTableRocksDB│ ...        │  │
│  │  │    (sys)        │ │  (analytics)    │            │  │
│  │  └─────────────────┘ └─────────────────┘            │  │
│  └──────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

### 启动流程

```rust
use laoflchDB_rust::{service, access};
use laoflchDB_rust::server::LaoflchDBServer;
use laoflchDB_rust::service::{DatabaseService, DatabaseServiceImpl, SchemaManager};
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let config = DatabaseConfig::load_or_default();
    
    // 1. 创建 Service 层 (异步)
    let service_layer: Arc<dyn DatabaseService> = Arc::new(
        DatabaseServiceImpl::new(&config.db_path).await
    );

    // 2. 创建 Access 层
    let access_service = Arc::new(AccessService::new(Arc::clone(&service_layer)));

    // 3. 创建并启动 LaoflchDBServer (异步)
    let server = LaoflchDBServer::new(
        Arc::new(SchemaManager::new(&config.db_path).await),
        service_layer,
        access_service,
    ).await;
    
    server.start(&config).await;
}
```

---

## 3. SchemaManager 设计 (Service 层)

`SchemaManager` 在 **Service 层** 实现，负责管理多个 Schema (RocksDB 实例)。**所有方法均为异步，使用 `tokio::sync::Mutex` 确保异步上下文中的线程安全**：

```rust
use tokio::sync::Mutex;

pub struct SchemaManager {
    engines: tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<MultiTableRocksDBEngine>>>>,
    base_path: String,
}

impl SchemaManager {
    pub async fn new(base_path: &str) -> Self;
    
    pub async fn get_schema_engine(&self, schema: &str) 
        -> Result<Arc<tokio::sync::Mutex<MultiTableRocksDBEngine>>, Box<dyn std::error::Error + Send + Sync>>;
    
    pub async fn list_schemas(&self) -> Vec<String>;
    
    pub async fn create_schema(&self, schema: &str) 
        -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    pub async fn drop_schema(&self, schema: &str) 
        -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}
```

### 工作原理

1. **延迟加载**：首次访问某个 Schema 时才创建对应的 RocksDB 实例
2. **缓存管理**：已打开的 Schema 保存在内存中，下次访问直接返回
3. **路径映射**：`{base_path}/{schema_name}` 对应每个 Schema 的 RocksDB 目录

---

## 4. DBEngine 接口和实现 (独立 crate)

### DBEngine Trait (laoflchdb_engines crate)

**位置**: [laoflchdb_engines/src/lib.rs](laoflchdb_engines/src/lib.rs)

**所有方法均为异步，使用 `#[async_trait]` 宏实现**：

```rust
#[async_trait::async_trait]
pub trait DBEngine: Send + Sync + 'static {
    async fn create_table(&mut self, table: &str, columns: &[(u32, &str, ColumnType)]) -> Result<u64, ...>;
    async fn drop_table(&mut self, table: &str) -> Result<(), ...>;
    async fn list_tables(&self) -> Result<Vec<String>, ...>;
    
    async fn put(&mut self, table: &str, key: &[u8], value: &[u8]) -> Result<(), ...>;
    async fn get(&self, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, ...>;
    async fn delete(&mut self, table: &str, key: &[u8]) -> Result<(), ...>;
    
    async fn get_table_meta(&self, table: &str) -> Result<Option<TableMeta>, ...>;
    
    // Query 接口 - 支持 CNF 查询和列投影下推
    async fn query(&self, query: &Query) -> Result<QueryResult, Box<dyn std::error::Error + Send + Sync>>;
    
    fn get_schema_name(&self) -> &str;
}
```

### MultiTableRocksDBEngine (multi_table_rocksdb crate)

**位置**: [multi_table_rocksdb/src/multi_table_rocksdb.rs](multi_table_rocksdb/src/multi_table_rocksdb.rs)

`DBEngine` trait 的 RocksDB 实现，**一个实例对应一个 Schema 和一个 RocksDB DB 实例**：

```rust
#[async_trait::async_trait]
impl DBEngine for MultiTableRocksDBEngine {
    // 所有 DBEngine trait 方法的异步实现
    async fn create_table(&mut self, table: &str, columns: &[(u32, &str, ColumnType)]) -> Result<u64, ...> {
        // 创建表逻辑...
    }
    // ... 其他异步方法
}

pub struct MultiTableRocksDBEngine {
    db: DB,              // 一个 RocksDB DB 实例
    schema_name: String, // 对应一个 Schema
}

impl MultiTableRocksDBEngine {
    pub fn new(options: &EngineOptions) -> Result<Self, ...> {
        // 初始化 RocksDB，创建默认表等
    }
}
```

### EngineOptions

```rust
pub struct EngineOptions {
    pub db_path: String,      // RocksDB 数据目录
    pub schema_name: String,  // Schema 名称
}

impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            db_path: "./db_data".to_string(),
            schema_name: "default".to_string(),
        }
    }
}
```

---

## 5. Service 层 (服务层)

### DatabaseService Trait

**所有方法均为异步，使用 `#[async_trait]` 宏实现**：

```rust
#[async_trait::async_trait]
pub trait DatabaseService: Send + Sync + 'static {
    async fn init_database(&self) -> Result<(), ...>;
    async fn create_schema(&self, schema: &str) -> Result<(), ...>;
    async fn list_schemas(&self) -> Result<Vec<String>, ...>;
    async fn drop_schema(&self, schema: &str) -> Result<(), ...>;
    
    async fn put(&self, schema: &str, table: &str, key: &[u8], value: &[u8]) -> Result<(), ...>;
    async fn get(&self, schema: &str, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, ...>;
    async fn delete(&self, schema: &str, table: &str, key: &[u8]) -> Result<(), ...>;
    async fn create_table(&self, schema: &str, table: &str, columns: &[...]) -> Result<u64, ...>;
    async fn list_tables(&self, schema: &str) -> Result<Vec<String>, ...>;
    async fn get_table_meta(&self, schema: &str, table: &str) -> Result<Option<TableMeta>, ...>;
    async fn sql_query(&self, schema: &str, sql: &str) -> Result<QueryResult, ...>;
}
```

### DatabaseServiceImpl

```rust
pub struct DatabaseServiceImpl {
    schema_manager: Arc<SchemaManager>,
    sql_engine: Arc<tokio::sync::RwLock<DataFusionSQLEngine<MultiTableRocksDBEngine>>>,
    default_schema: String,
}

impl DatabaseServiceImpl {
    pub async fn new(base_path: &str) -> Self {
        let schema_manager = SchemaManager::new(base_path).await;
        Self { 
            schema_manager: Arc::new(schema_manager),
            sql_engine: Arc::new(tokio::sync::RwLock::new(DataFusionSQLEngine::new())),
            default_schema: "sys".to_string(),
        }
    }
}
```

---

## 6. Access 层 (接入服务层)

位置: [src/access/mod.rs](src/access/mod.rs)

### 设计目标

- 负责 gRPC 和 REST API 访问的接入、注册和路由
- 对接 Service 层的服务接口
- 支持多种协议扩展

### AccessService

```rust
pub struct AccessService {
    service: Arc<dyn DatabaseService>,
}

impl AccessService {
    pub fn new(service: Arc<dyn DatabaseService>) -> Self;
    pub fn get_grpc_service(&self) -> GrpcService;
    pub fn get_rest_service(&self) -> RestService;
}
```

### GrpcService

gRPC 协议的具体实现。

### RestService

REST API 协议的具体实现，基于 Axum 框架。

**位置**: [src/access/rest.rs](src/access/rest.rs)

---

## 7. 字段类型封装

### Field Proto 定义

```protobuf
message Field {
    oneof value {
        String string_value = 1;
        Integer integer_value = 2;
        Bytes bytes_value = 3;
        Float float_value = 4;
        List list_value = 5;
        Image image_value = 6;
    }
}

message String { string value = 1; }
message Integer { int64 value = 1; }
message Bytes { bytes value = 1; }
message Float { double value = 1; }
message List { repeated bytes items = 1; }
message Image { bytes data = 1; string format = 2; }
```

### ColumnType 枚举

```protobuf
enum ColumnType {
    COLUMN_TYPE_STRING = 0;
    COLUMN_TYPE_INT64 = 1;
    COLUMN_TYPE_BYTES = 2;
    COLUMN_TYPE_FLOAT = 3;
    COLUMN_TYPE_LIST = 4;
    COLUMN_TYPE_IMAGE = 5;
}
```

---

## 8. API 接口

### gRPC RPC 服务接口

定义: [src/access/proto/rpc.proto](src/access/proto/rpc.proto)

```protobuf
service LaoflchDb {
    // 用户认证接口
    rpc Login (LoginRequest) returns (LoginResponse);
    rpc Logout (LogoutRequest) returns (LogoutResponse);
    
    // 数据操作接口
    rpc Get (GetRequest) returns (GetResponse);
    rpc Put (PutRequest) returns (PutResponse);
    rpc Delete (DeleteRequest) returns (DeleteResponse);
    rpc CreateTable (CreateTableRequest) returns (CreateTableResponse);
    rpc ListTables (ListTablesRequest) returns (ListTablesResponse);
    rpc GetTableMeta (GetTableMetaRequest) returns (GetTableMetaResponse);
    rpc SqlQuery (SqlQueryRequest) returns (SqlQueryResponse);
}
```

### REST API 接口

详细文档: [REST_API.md](REST_API.md)

| 端点 | 方法 | 说明 | 是否需要认证 |
|------|------|------|-------------|
| `/health` | GET | 健康检查 | 否 |
| `/api/v1/login` | POST | 用户登录 | 否 |
| `/api/v1/logout` | POST | 用户登出 | 否 |
| `/api/v1/tables` | POST | 创建表 | 是 |
| `/api/v1/schemas/{schema}/tables` | GET | 列出表 | 是 |
| `/api/v1/schemas/{schema}/tables/{table}` | GET | 获取表元数据 | 是 |
| `/api/v1/put` | POST | 插入数据 | 是 |
| `/api/v1/get` | GET | 读取数据 | 是 |
| `/api/v1/delete` | POST | 删除数据 | 是 |
| `/api/v1/sql_query` | POST | SQL 查询 | 是 |
| `/api/v1/object-store` | GET | 列出所有 Bucket（S3 兼容） | 是 |
| `/api/v1/object-store/{bucket}` | PUT | 创建 Bucket | 是 |
| `/api/v1/object-store/{bucket}` | GET | 列出 Bucket 中的对象 | 是 |
| `/api/v1/object-store/{bucket}` | DELETE | 删除 Bucket | 是 |
| `/api/v1/object-store/{bucket}/{key}` | PUT | 上传对象 | 是 |
| `/api/v1/object-store/{bucket}/{key}` | GET | 下载对象 | 是 |
| `/api/v1/object-store/{bucket}/{key}` | HEAD | 获取对象元数据 | 是 |
| `/api/v1/object-store/{bucket}/{key}` | DELETE | 删除对象 | 是 |

### 认证机制

LaoflchDB 使用 Token 认证机制：

1. **获取 Token**: 通过登录接口获取认证 Token
2. **使用 Token**: 在请求头中携带 `Authorization: Bearer <token>`
3. **Token 有效期**: 默认 24 小时
4. **默认用户**: 初始化时自动创建 `admin` 用户（密码: `laoflchdb`）

**认证流程**:
```
客户端 → POST /api/v1/login → 获取 Token → 请求携带 Token → 受保护的 API
```

---

## 9. YAML 配置文件

配置文件 `laoflchdb.yaml`：

```yaml
# laoflchDB 配置文件
db_path: ./laoflch_db_data    # 数据库根目录 (包含多个 Schema)
log_level: info              # 日志级别

# 默认权限策略 (当 service_id 没有特定权限配置时使用)
default_policy: allow

# 访问协议配置 (每个协议有独立的 service_id 和端口)
access_protocols:
  - protocol: grpc
    enabled: true
    addr: 127.0.0.1:19777
    service_id: grpc_admin

  - protocol: rest
    enabled: true
    addr: 127.0.0.1:8080
    service_id: rest_admin

# 权限配置 (每个 service_id 独立配置)
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
      - sql_query

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
      - sql_query
```

---

## 10. lsql 命令行客户端

`lsql` 是一个类似 PostgreSQL psql 的交互式 SQL 客户端，通过 gRPC 连接到 laoflchDB。

### 安装和编译

```bash
cargo build --bin lsql
```

### 命令行参数

```bash
lsql --help

lsql 命令行选项：
      --host <HOST>          数据库服务器地址，格式为 host:port (必需)
  -s, --schema <SCHEMA>      默认 Schema 名称 (默认: sys)
  -c, --command <COMMAND>    执行单次 SQL 命令后退出
  -h, --help                 显示帮助信息
```

### 使用示例

```bash
# 连接到本地数据库，使用默认 sys schema
lsql --host 127.0.0.1:19777

# 连接到指定服务器，使用特定 schema
lsql --host 192.168.1.100:19777 --schema analytics

# 执行单次 SQL 查询并退出
lsql --host 127.0.0.1:19777 --command "SELECT * FROM users"
```

### 交互式命令

进入交互式模式后，可以使用以下命令：

| 命令 | 说明 |
|------|------|
| `\q` 或 `\quit` | 退出 lsql |
| `\help` 或 `\?` | 显示帮助信息 |
| `\dn` 或 `\schemas` | 列出所有可用的 Schema |
| `\dt` | 列出当前 Schema 中的所有表 |
| `\c <schema>` 或 `\connect <schema>` | 切换到指定的 Schema |
| `<SQL语句>` | 执行 SQL 查询 |

### 交互式会话示例

```sql
lsql@sys> \dn
所有 Schema:
  - sys
  - analytics
lsql@sys> \c analytics
已切换到 Schema 'analytics'
lsql@analytics> \dt
当前 Schema 'analytics' 中的表:
  - events
  - logs
lsql@analytics> SELECT * FROM events LIMIT 5;
...
```

## 11. 命令行 CLI 能力

```
Commands:
  start      # 以 standalone daemon 方式启动服务 (gRPC + REST)
    Options:
      -c, --config <CONFIG>   配置文件路径
      -a, --addr <ADDR>       gRPC bind address
      -d, --db-path <DB_PATH> 数据库根目录

  init       # 初始化数据库 (创建 sys schema 和 user 表)
```

### 启动示例

```bash
# 编译
cargo build --release

# 初始化数据库
./target/release/laoflchDB-rust init

# 启动服务 (使用配置文件)
./target/release/laoflchDB-rust -c laoflchdb.yaml start

# 启动服务 (指定参数)
./target/release/laoflchDB-rust start --db-path ./data
```

---

## 11. RocksDB ldb 工具

项目同时编译 RocksDB 原生的 `ldb` 工具，用于直接管理和调试 RocksDB 数据：

### 编译 ldb

```bash
# 使用 cargo xtask 编译
cargo ldb          # 仅编译 ldb 工具
cargo all          # 编译所有 (Rust + ldb)
```

### ldb 使用示例

```bash
# 列出列族
./target/release/ldb list_column_families --db=./laoflch_db_data/sys

# 扫描数据
./target/release/ldb scan --db=./laoflch_db_data/sys --column_family=user

# 交互式查询
./target/release/ldb query --db=./laoflch_db_data/sys
```

### xtask 任务

| 命令 | 说明 |
|------|------|
| `cargo ldb` | 仅编译 ldb 工具 |
| `cargo all` | 编译所有 (Rust + ldb) |
| `cargo auto-test` | 运行自动回归测试 |

---

## 12. 核心依赖

| Rust Crate | 版本 | 用途 |
|------------|------|------|
| rust-rocksdb | 0.50 | KV 存储 (RocksDB v11.1.1) |
| tonic | 0.11 | gRPC HTTP/2 async 服务 |
| axum | 0.7 | REST API 服务框架 |
| prost | 0.12 | protobuf 编解码 |
| protobuf | 3.7 | protobuf 编解码 (SQL 引擎) |
| clap | 4.5 | 命令行参数 |
| tokio | 1.0 | async runtime |
| tokio-sync | 1.0 | 异步锁机制 |
| async-trait | 0.1 | 异步 trait 支持 |
| serde | 1.0 | YAML 配置序列化 |
| serde_yaml | 0.9 | YAML 解析 |
| datafusion | 53.1.0 | SQL 解析和查询引擎 |
| arrow | 58.3.0 | 列式数据格式 |
| arrow-schema | 58.3.0 | Arrow Schema 定义 |
| arrow-array | 58.3.0 | Arrow Array 实现 |
| futures | 0.3 | 异步流处理 |
| snowflake_me | 0.5 | Snowflake ID 生成 |
| tantivy | 0.26 | 全文索引引擎 |
| candle-core | 0.10 | 深度学习推理框架 (CPU/CUDA) |
| candle-nn | 0.10 | 神经网络模块 (Candle) |
| candle-kernels | 0.10 | CUDA 内核加速 (Candle) |

---

## 13. 快速开始

### 1. 编译

```bash
# 编译 Rust 代码
cargo build --release
```

### 2. 初始化数据库

```bash
./target/release/laoflchDB-rust init
```

**幂等性说明：**

- **`sys` Schema**：采用"存在则跳过"策略，不会删除或修改现有数据
- **`--example` 选项**：初始化示例数据，会删除并重建 `example` Schema

```bash
# 初始化数据库（幂等，不会删除现有数据）
./target/release/laoflchDB-rust init

# 初始化数据库并创建示例数据（会删除重建 example Schema）
./target/release/laoflchDB-rust init --example
```

### 3. 启动服务

```bash
./target/release/laoflchDB-rust -c laoflchdb.yaml start
```

服务将同时启动：
- **gRPC 服务**: http://127.0.0.1:19777
- **REST API**: http://127.0.0.1:8080

### 4. 测试 SQL 查询

```bash
# 创建测试表
curl -X POST http://localhost:8080/api/v1/tables \
  -H "Content-Type: application/json" \
  -d '{
    "schema": "sys",
    "table_name": "test_users",
    "columns": [
      {"name": "id", "column_type": "INT64"},
      {"name": "name", "column_type": "STRING"},
      {"name": "age", "column_type": "INT64"}
    ]
  }'

# 插入数据
curl -X POST http://localhost:8080/api/v1/schemas/sys/tables/test_users/rows \
  -H "Content-Type: application/json" \
  -d '{"row": {"row_type": 0, "version": 1, "data": ["1", "Alice", "30"]}}'

# SQL 查询
curl -X POST http://localhost:8080/api/v1/sql_query \
  -H "Content-Type: application/json" \
  -d '{"sql": "SELECT id, name, age FROM test_users WHERE age > 25"}'
```

### 5. 作为库使用

如果你想将 laoflchDB 作为 Rust 库使用：

```rust
use laoflchDB_rust::service::{DatabaseService, DatabaseServiceImpl};
use laoflchDB_rust::db_engine::pb::ColumnType;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    // 1. 创建 DatabaseService (异步初始化)
    let service: Arc<dyn DatabaseService> = Arc::new(
        DatabaseServiceImpl::new("./my_db_data").await
    );

    // 2. 初始化数据库
    service.init_database().await.unwrap();

    // 3. 创建表 (异步调用)
    let columns = vec![
        (0, "id", ColumnType::Int64),
        (1, "name", ColumnType::String),
    ];
    
    let table_id = service.create_table("sys", "my_table", &columns).await.unwrap();
    println!("Created table with id: {}", table_id);

    // 4. SQL 查询 (异步调用)
    let result = service.sql_query("sys", "SELECT * FROM my_table").await.unwrap();
    println!("Query result: {:?}", result);
}
```

---

## 14. 测试

### 测试套件

项目包含完整的测试套件：

| 测试类型 | 位置 | 说明 |
|---------|------|------|
| Rust 单元测试 | [tests/](tests/) | 基础功能和 API 测试 |
| 集成测试 | [tests/integration_tests.rs](tests/integration_tests.rs) | SQL 查询下推测试 |
| REST API 测试 | [tests_python/test_sql_query_validation.py](tests_python/test_sql_query_validation.py) | SQL 查询验证 |
| gRPC API 测试 | [tests_python/test_grpc_sql_query.py](tests_python/test_grpc_sql_query.py) | gRPC SQL 查询测试 |

### 运行测试

```bash
# 运行所有 Rust 测试
./run_tests.sh

# 运行完整测试套件 (Rust + Python)
./run_all_tests.sh

# 快速验证
./verify_tests.sh

# 运行 Python SQL 查询验证测试
python3 tests_python/test_sql_query_validation.py
```

---

## 17. 架构升级说明

### SQL 引擎重构

**DataFusionStorageEngine** 已从 `laoflchdb_engines` 迁移到 `laoflchdb_sql_df_engine`：

| 变更项 | 说明 |
|--------|------|
| `DataFusionStorageEngine` Trait | 定义在 `laoflchdb_sql_df_engine` 中 |
| `RocksDBTable` | 拆分为独立文件 `rocksdb_table.rs` |
| `RocksScanExec` | 自定义物理执行算子，替代 MemTable |
| 查询下推 | 支持 Filter/Project/Limit 下推到存储层 |
| 逻辑表达式 | 支持 AND/OR 条件下推 |

### 依赖关系

```
laoflchdb_sql_df_engine
    ├── laoflchdb_engines (SQLEngine, StorageEngine, QueryResult)
    └── datafusion (SQL 解析和优化)

multi_table_rocksdb
    ├── laoflchdb_engines (DBEngine, ColumnType)
    ├── laoflchdb_sql_df_engine (DataFusionStorageEngine)
    └── datafusion (TableProvider, ExecutionPlan)
```

---

### 0.1.9 (当前)
- **对象存储服务 (ObjectStoreService)**: 新增 S3 兼容的对象存储服务，基于 RocksDB BlobDB 实现大对象存储
  - 完整的 gRPC API：PutObject/GetObject/DeleteObject/ListObjects/HeadObject/CopyObject/DeleteObjects/CreateBucket/DeleteBucket/ListBuckets
  - S3 兼容的 REST API：`/api/v1/object-store` 前缀下的 Bucket/Object HTTP 端点
  - 支持大对象存储（BlobDB，默认单文件最大 256MB，zstd 压缩）
  - 支持对象元数据管理（content_type、etag、last_modified、用户自定义元数据）
  - 支持目录结构模拟（通过 delimiter + prefix 实现 S3 风格的 common_prefixes）
  - 支持跨 Bucket 复制（CopyObject）和批量删除（DeleteObjects）
  - 所有 Bucket/Object 操作均为幂等操作
- **KV RocksDB 引擎扩展**: 在 `laoflchdb_kv_rocksdb_engine` 中新增 BlobDB 支持（`BlobDBConfig` 配置和 `new_with_blob_db` 构造方法）
- **REST 路由集成**: 通过 `RestService::with_object_store_router()` 将对象存储 REST 路由挂载到主服务器
- **测试覆盖**: 新增 `test_object_store_service_grpc.py`（29 个场景）和 `test_object_store_service_rest.py`（29 个场景，S3 兼容性测试）

### 0.1.4
- **向量化服务 (VectorService)**: 基于 Candle 0.10 + CUDA 实现文本和图片向量化推理，支持 BERT/XLM-RoBERTa/ViT 模型
- **嵌入向量索引服务 (EmbeddingIndexService)**: 基于 HNSW 算法实现近似最近邻搜索，支持向量持久化和快照管理
- **表和字段注释支持**: 在 `TableMeta` 和 `ColumnMeta` 中添加了 `comment` 字段，支持语义化注释
- **订单交易系统示例**: `--example` 初始化时创建完整的订单交易系统表结构，包含表和字段注释
- **lsql 命令行客户端**: 类似 PostgreSQL psql 的交互式 SQL 客户端，支持 `\v` 作为 `\version` 的别名
- **ListSchemas API**: 新增 gRPC API 用于列出所有 Schema
- **execute_query 日志**: 添加详细的 SQL 执行日志输出
- **错误处理优化**: SQL 执行错误时不退出进程
- **Schema 验证**: 切换和默认 Schema 时验证是否存在
- **优雅关闭功能**: 支持 SIGINT/SIGTERM 信号处理，自动刷新 RocksDB 数据并释放锁文件

---

## 18. 订单交易系统示例数据

使用 `--example` 参数初始化数据库时，会创建完整的订单交易系统表结构和万级样例数据：

### 表结构设计

| 表名 | 字段 | 类型 | 说明 |
|------|------|------|------|
| **customers** | id | INT64 | 客户ID |
| | name | STRING | 客户名称 |
| | email | STRING | 邮箱 |
| | phone | STRING | 手机号 |
| | address | STRING | 地址 |
| | created_at | STRING | 创建时间 |
| **products** | id | INT64 | 产品ID |
| | name | STRING | 产品名称 |
| | price | FLOAT | 价格 |
| | stock | INT64 | 库存 |
| | category | STRING | 分类 |
| | description | STRING | 描述 |
| **orders** | id | INT64 | 订单ID |
| | customer_id | INT64 | 客户ID |
| | order_date | STRING | 下单时间 |
| | total_amount | FLOAT | 订单总额 |
| | status | STRING | 状态 |
| | shipping_address | STRING | 配送地址 |
| **order_items** | id | INT64 | 明细ID |
| | order_id | INT64 | 订单ID |
| | product_id | INT64 | 产品ID |
| | quantity | INT64 | 数量 |
| | unit_price | FLOAT | 单价 |
| | discount | FLOAT | 折扣 |

### 样例数据规模

| 表名 | 数据量 |
|------|--------|
| customers | 1000 条 |
| products | 100 条 (5个分类) |
| orders | 10000 条 |
| order_items | ~20000 条 |

### 使用示例

```bash
# 初始化 example 库（会删除并重建）
./target/release/laoflchDB-rust init --example --db-path ./test_db

# 启动服务后通过 lsql 查询
lsql --host 127.0.0.1:19777 --schema example

lsql@example> SELECT COUNT(*) FROM customers;   -- 1000
lsql@example> SELECT COUNT(*) FROM orders;      -- 10000
lsql@example> SELECT c.name, o.order_date, o.total_amount 
             FROM orders o 
             JOIN customers c ON o.customer_id = c.id 
             LIMIT 10;
```

---

## 19. 版本历史

### 0.1.2
- **SQL 查询下推优化**: 支持 Filter、Project、Limit 下推
- **自定义物理执行算子**: `RocksScanExec` 直接对接 RocksDB
- **逻辑表达式支持**: AND/OR 条件下推
- **数据类型正确返回**: INT64、STRING、FLOAT、BYTES
- **代码重构**: `RocksDBTable` 拆分为独立文件
- **文档更新**: 更新 README 和 design.md

### 0.1.1
- 支持存储格式改为 protobuf Field 对象
- 实现完整的数据类型映射
- SQL 查询返回正确的数据类型
- 修复谓词下推的比较逻辑
- 添加 Python 自动回归测试

### 0.1.0
- 实现基础 SQL 查询引擎
- 添加 filter、project、limit 下推优化
- 实现自定义物理执行算子 RocksScanExec
- 完善异步架构设计
