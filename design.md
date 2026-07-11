# laoflchDB-rust 详细设计文档

---

## 目录

1. [总体架构设计](#1-总体架构设计)
2. [Schema 与 RocksDB 映射设计](#2-schema-与-rocksdb-映射设计)
3. [LaoflchDBServer 总入口架构](#3-laoflchdbserver-总入口架构)
4. [SchemaManager 设计 (Service 层)](#4-schemamanager-设计-service-层)
5. [DBEngine 接口和实现](#5-dbengine-接口和实现)
6. [Service 层设计](#6-service-层设计)
7. [Access 层设计](#7-access-层设计)
8. [SQL 引擎设计](#8-sql-引擎设计)
9. [Query 接口设计](#9-query-接口设计)
10. [异步调用设计](#10-异步调用设计)
11. [前缀过滤与 Snowflake ID 设计](#11-前缀过滤与-snowflake-id-设计)
12. [全文索引引擎设计](#12-全文索引引擎设计)
13. [向量化服务设计 (VectorService)](#13-向量化服务设计-vectorservice)
14. [嵌入向量索引服务设计 (EmbeddingIndexService)](#14-嵌入向量索引服务设计-embeddingindexservice)
15. [核心依赖](#15-核心依赖)
16. [架构设计原则](#16-架构设计原则)
17. [版本历史](#17-版本历史)

---

## 1. 总体架构设计

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

### 目录结构源码树：

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
│   ├── test_sql_query_validation.py
│   ├── test_grpc_sql_query.py
│   ├── test_vector_service_grpc.py   # 向量化服务测试
│   ├── test_embedding_service_grpc.py # 嵌入向量索引服务测试
│   ├── test_index_grpc.py            # 全文索引 gRPC 测试
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

## 2. Schema 与 RocksDB 映射设计

### 2.1 核心映射关系

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

### 2.2 Column Family 设计

| Column Family | 用途 |
|---------------|------|
| `default` | 存储 db、table、col 级别的元数据 |
| `{table_name}` | 存储对应表的数据 |

### 2.3 元数据 Key 格式

| 类型 | Key 格式 | 示例 |
|------|----------|------|
| Schema 元数据 | `META-SCHEMA:{schema_name}` | `META-SCHEMA:sys` |
| 表元数据 | `META-TABLE:{table_name}:{table_id}` | `META-TABLE:user:0` |
| 字段元数据 | `META-COL:{table_id_fixed}:{column_name}:{column_id}:{column_type}` | `META-COL:00000000000000000000:user_id:0:COLUMN_TYPE_INT64` |

**注意**: 其中 `{table_id_fixed}` 是固定长度 20 字符的字符串，不足时前面补0。

### 2.4 自增 ID 设计

| ID 类型 | 起始值 | 存储位置 | 说明 |
|---------|--------|----------|------|
| `table_id` | 0 | `SchemaMeta.next_auto_inc_table_id` | Schema 内唯一，每创建表 +1 |
| `column_id` | 0 | `TableMeta.next_auto_inc_column_id` | 表内唯一，每添加字段 +1 |

### 2.5 MAX_TABLE_ID_LENGTH 常量

在 `multi_table_rocksdb` crate 中定义，用于固定长度的 table_id 字符串格式：

```rust
pub const MAX_TABLE_ID_LENGTH: usize = 20;
```

---

## 3. LaoflchDBServer 总入口架构

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

### 3.1 启动流程

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

## 4. SchemaManager 设计 (Service 层)

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

### 4.1 工作原理

1. **延迟加载**：首次访问某个 Schema 时才创建对应的 RocksDB 实例
2. **缓存管理**：已打开的 Schema 保存在内存中，下次访问直接返回
3. **路径映射**：`{base_path}/{schema_name}` 对应每个 Schema 的 RocksDB 目录

---

## 5. DBEngine 接口和实现

### 5.1 DBEngine Trait (laoflchdb_engines crate)

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

### 5.2 MultiTableRocksDBEngine (multi_table_rocksdb crate)

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

### 5.3 EngineOptions

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

## 6. Service 层设计

### 6.1 DatabaseService Trait

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

### 6.2 DatabaseServiceImpl

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

## 7. Access 层设计

### 7.1 设计目标

- 负责 gRPC 和 REST API 访问的接入、注册和路由
- 对接 Service 层的服务接口
- 支持多种协议扩展

### 7.2 AccessService

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

### 7.3 GrpcService

gRPC 协议的具体实现。

### 7.4 RestService

REST API 协议的具体实现，基于 Axum 框架。

**位置**: [src/access/rest.rs](src/access/rest.rs)

---

## 8. SQL 引擎设计

### 8.1 架构概述

SQL 引擎基于 **DataFusion + Arrow + RocksDB** 实现，提供 SQL 查询能力。架构采用双层接口设计，支持 **filter、project、limit 下推** 优化。

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         LaoflchDBServer                              │
│                                      │                               │
│                                      ▼                               │
│  ┌──────────────────────────────────────────────────────────────────┐  │
│  │                      SQLEngine (SQL 查询接口)                   │  │
│  │  ┌────────────────────────────────────────────────────────────┐ │  │
│  │  │            DataFusionSQLEngine<E>                        │ │  │
│  │  │  ┌──────────────────────────────────────────────────────┐│ │  │
│  │  │  │  DataFusion SessionContext                         ││ │  │
│  │  │  │  - SQL 解析 / 查询规划 / 查询优化                   ││ │  │
│  │  │  └──────────────────────────────────────────────────────┘│ │  │
│  │  │                          │                             │ │  │
│  │  │          ┌───────────────┴───────────────┐             │ │  │
│  │  │          ▼                               ▼             │ │  │
│  │  │  ┌─────────────────┐           ┌─────────────────────┐ │ │  │
│  │  │  │  StorageEngine  │           │DataFusionStorageEngine│ │ │  │
│  │  │  │  (通用存储接口) │           │  (SQL专用接口)      │ │ │  │
│  │  │  └────────┬────────┘           └───────────┬─────────┘ │ │  │
│  │  │           │                                  │         │ │  │
│  │  │           └─────────────────┬────────────────┘         │ │  │
│  │  │                             ▼                         │ │  │
│  │  │           ┌─────────────────────────────┐             │ │  │
│  │  │           │   MultiTableRocksDBEngine   │             │ │  │
│  │  │           │  (实现两个接口)              │             │ │  │
│  │  │           └─────────────────────────────┘             │ │  │
│  │  └────────────────────────────────────────────────────────┘ │  │
│  └──────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────┘
```

### 8.2 自定义物理执行算子 (RocksScanExec)

为避免使用 `MemTable` 包装查询结果，项目实现了自定义物理执行算子 `RocksScanExec`，直接对接存储引擎进行数据扫描和下推处理。

**位置**: [multi_table_rocksdb/src/rocksdb_table.rs](multi_table_rocksdb/src/rocksdb_table.rs)

**核心结构**:

```rust
struct RocksScanExec {
    engine: Arc<TokioRwLock<MultiTableRocksDBEngine>>,
    table_name: String,
    projection: Option<Vec<usize>>,      // 列投影下推
    filters: Vec<ColumnFilter>,           // 过滤条件下推
    limit: Option<usize>,                 // 限制条数下推
    schema: Arc<Schema>,
    properties: Arc<PlanProperties>,
}
```

**关键方法**:

```rust
impl ExecutionPlan for RocksScanExec {
    fn execute(&self, _partition: usize, _context: Arc<TaskContext>) 
        -> Result<SendableRecordBatchStream> {
        // 1. 创建独立线程执行查询
        // 2. 调用 table_to_arrow_with_pushdown 方法
        // 3. 返回异步流
    }
}
```

**执行流程**:

```
SQL 查询 → DataFusion 优化器 → TableProvider.scan() 
    → RocksScanExec (物理算子) 
    → table_to_arrow_with_pushdown() 
    → 存储引擎直接扫描
```

**优势**:
- 避免 MemTable 包装开销
- SQL 的 filter、project、limit 直接下推到存储引擎层
- 自定义物理算子直接对接 RocksDB

### 8.3 谓词下推支持 (supports_filters_pushdown)

**位置**: [multi_table_rocksdb/src/rocksdb_table.rs](multi_table_rocksdb/src/rocksdb_table.rs)

支持以下下推类型：

| 下推类型 | 说明 | 支持的操作符 |
|---------|------|-------------|
| `Exact` | 过滤器可以精确下推到存储层执行 | `=`, `!=`, `<`, `>`, `<=`, `>=` |
| `Exact` | 逻辑操作符支持 | `AND`, `OR` (所有子表达式都支持时) |
| `Unsupported` | 不支持下推的表达式 | 其他类型 |

**实现逻辑**:

```rust
fn supports_filters_pushdown(
    &self,
    filters: &[&datafusion::logical_expr::Expr],
) -> datafusion::error::Result<Vec<datafusion_expr::TableProviderFilterPushDown>> {
    // 判断单个表达式是否支持下推
    fn is_supported(expr: &datafusion::logical_expr::Expr) -> bool {
        match expr {
            Expr::BinaryExpr(BinaryExpr { op, left, right }) => match op {
                // AND 表达式：两个子表达式都支持才支持
                Operator::And => is_supported(left) && is_supported(right),
                // OR 表达式：两个子表达式都支持才支持
                Operator::Or => is_supported(left) && is_supported(right),
                // 支持的比较操作符
                Operator::Eq | Operator::NotEq | Operator::Lt | 
                Operator::Gt | Operator::LtEq | Operator::GtEq => true,
                _ => false,
            },
            _ => false,
        }
    }
    // ...
}
```

### 8.4 projected_columns 列投影下推

`Query` 结构体新增 `projected_columns` 字段，支持在查询时指定需要返回的列，实现列级别的数据过滤，减少不必要的数据加载。

**位置**: [laoflchdb_engines/proto/query.proto](laoflchdb_engines/proto/query.proto)

```protobuf
message Query {
    repeated TableFilter table_filters = 1;   // 多个表过滤器，AND 关系 (CNF)
    optional uint32 limit = 2;                // 返回结果数量限制
    optional uint32 offset = 3;               // 跳过的结果数量
    repeated string projected_columns = 4;     // 需要返回的列名列表，为空则返回所有列
}
```

**性能优势**:

| 优化点 | 说明 |
|--------|------|
| 减少 IO | 只读取需要的列，减少磁盘读取量 |
| 减少内存 | 只存储需要的列数据 |
| 加速处理 | 减少数据传输和处理时间 |

### 8.5 SQLEngine Trait 定义

**位置**: [laoflchdb_engines/src/lib.rs](laoflchdb_engines/src/lib.rs)

```rust
#[async_trait::async_trait]
pub trait SQLEngine: Send + Sync + 'static {
    async fn execute_query(&self, sql: &str) -> Result<QueryResult, Box<dyn std::error::Error + Send + Sync>>;
    
    async fn register_table(&mut self, table_name: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    async fn refresh_tables(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}
```

---

## 9. Query 接口设计

### 9.1 接口定义

`Query` 接口定义了服务层提交到 DBEngine 的数据查询信息，支持 **CNF (Conjunctive Normal Form) 表达式** 查询，并支持 **列投影下推**。

**位置**: [laoflchdb_engines/proto/query.proto](laoflchdb_engines/proto/query.proto)

```protobuf
message Query {
    repeated TableFilter table_filters = 1;  // 多个表过滤器，AND 关系 (CNF)
    optional uint32 limit = 2;               // 返回结果数量限制
    optional uint32 offset = 3;              // 跳过的结果数量
    repeated string projected_columns = 4;    // 需要返回的列名列表，为空则返回所有列
}

message TableFilter {
    string table_name = 1;                    // 表名
    repeated ColumnFilter column_filters = 2; // 多个列过滤器，AND 关系
}

message ColumnFilter {
    string column_name = 1;                     // 列名
    repeated ColumnFilterCondition conditions = 2; // 多个条件，OR 关系
}

message ColumnFilterCondition {
    FilterOperator op = 1;       // 操作符
    optional Field value = 2;    // 单值条件
    repeated Field values = 3;   // 多值条件 (用于 IN/NOT_IN)
}

enum FilterOperator {
    FILTER_OPERATOR_UNSPECIFIED = 0;
    FILTER_OPERATOR_EQ = 1;      // 等于
    FILTER_OPERATOR_NEQ = 2;     // 不等于
    FILTER_OPERATOR_GT = 3;      // 大于
    FILTER_OPERATOR_GTE = 4;     // 大于等于
    FILTER_OPERATOR_LT = 5;      // 小于
    FILTER_OPERATOR_LTE = 6;     // 小于等于
    FILTER_OPERATOR_IN = 7;      // IN 列表
    FILTER_OPERATOR_NOT_IN = 8;  // NOT IN 列表
    FILTER_OPERATOR_IS_NULL = 9; // 为空
    FILTER_OPERATOR_IS_NOT_NULL = 10; // 不为空
}

message QueryResult {
    repeated QueryRow rows = 1;
    repeated string columns = 2;  // 返回的列名列表
}

message QueryRow {
    string table_name = 1;
    uint64 row_id = 2;
    optional Row row = 3;
}
```

### 9.2 CNF 表达式结构

Query 接口的逻辑关系符合 **CNF (Conjunctive Normal Form)** 表达式：

```
Query = (TableFilter_1 AND TableFilter_2 AND ...)
TableFilter = (ColumnFilter_1 AND ColumnFilter_2 AND ...)
ColumnFilter = (Condition_1 OR Condition_2 OR ...)
```

### 9.3 存储层过滤逻辑

**位置**: [multi_table_rocksdb/src/multi_table_rocksdb.rs](multi_table_rocksdb/src/multi_table_rocksdb.rs)

```rust
fn check_table_filters(
    &self, 
    row: &Row, 
    column_filters: &[ColumnFilter], 
    columns: &std::collections::HashMap<String, (u64, ColumnType)>
) -> bool {
    for column_filter in column_filters {
        if !self.check_column_filter(row, column_filter, columns) {
            return false;  // 不同列之间是 AND 关系
        }
    }
    true
}

fn check_column_filter(
    &self, 
    row: &Row, 
    column_filter: &ColumnFilter, 
    columns: &std::collections::HashMap<String, (u64, ColumnType)>
) -> bool {
    for condition in &column_filter.conditions {
        if self.check_column_condition(row, column_filter, condition, columns) {
            return true;  // 同一列的多个条件之间是 OR 关系
        }
    }
    false
}
```

### 9.4 DBEngine Query 方法

```rust
#[async_trait::async_trait]
pub trait DBEngine: Send + Sync + 'static {
    // Query 接口 - 支持 CNF 查询
    async fn query(&self, query: &Query) -> Result<QueryResult, Box<dyn std::error::Error + Send + Sync>>;
    // ... 其他方法
}
```

---

## 10. 异步调用设计

laoflchDB 分层架构之间通过 **async/await** 和 **tokio** 运行时实现完全异步调用。

### 10.1 核心技术

| 技术 | 说明 |
|------|------|
| `#[async_trait]` | 为 trait 提供 async fn 支持 |
| `#[tonic::async_trait]` | 为 gRPC 服务提供 async fn 支持 |
| `tokio::sync::Mutex` | 异步上下文中的线程安全锁 |
| `Arc<dyn Trait>` | 线程安全的共享所有权 |
| `Box<dyn Error + Send + Sync>` | 跨线程的错误传递 |
| `tokio` | 异步运行时 |
| `tokio::spawn` | 异步任务并发执行 |

### 10.2 异步 Mutex 锁设计

**关键设计**：使用 `tokio::sync::Mutex` 而非 `std::sync::Mutex`

- **问题**：`std::sync::Mutex` 在持有锁期间不能跨 await 点
- **解决方案**：使用 `tokio::sync::Mutex`，其 `.lock().await` 返回 `MutexGuard` 可以安全跨 await

```rust
use tokio::sync::Mutex;

pub struct SchemaManager {
    // 使用 tokio::sync::Mutex
    engines: tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<MultiTableRocksDBEngine>>>>,
}

impl SchemaManager {
    pub async fn get_schema_engine(&self, schema: &str) -> Result<...> {
        // 获取锁 - await 释放控制权
        let engine = self.engines.lock().await;
        // 持有锁期间可以 await 多次
        engine.create_table(&table, &columns).await
    }
}
```

### 10.3 异步调用链路

```
gRPC/REST 请求
    ↓
Access 层 (GrpcService/RestService)
    ↓ .await (异步调用)
Service 层 (DatabaseService + SchemaManager)
    ↓ .lock().await (异步锁)
StorageEngine 层 (MultiTableRocksDBEngine)
    ↓ .await (异步方法调用)
RocksDB 存储
```

### 10.4 SQL 查询调用链路

```
SQL 查询请求
    ↓
Service 层 (DatabaseService.sql_query)
    ↓ .read().await (异步读锁)
SQLEngine 层 (DataFusionSQLEngine)
    ↓ .await (异步执行)
DataFusion SessionContext
    ↓ TableProvider.scan()
    ↓ RocksScanExec.execute()
    ↓ table_to_arrow_with_pushdown()
    ↓ RecordBatch 结果
    ↓ 转换为 QueryResult
```

---

## 11. 前缀过滤与 Snowflake ID 设计

### 11.1 Row ID 生成规则

multi_table 的 `row_id` 生成规则为：

1. **Snowflake 算法生成唯一 ID**
2. **转换为大端字节序 (Big Endian) 存储**

**核心优势**：
- **RocksDB 保序**：大端字节序确保 ID 在 RocksDB 中按时间顺序排序
- **前缀过滤**：时间戳作为 ID 的高位前缀，支持高效的时间范围扫描

### 11.2 Snowflake ID 结构

Snowflake ID 是一个 64 位整数，结构如下：

| 位范围 | 位数 | 说明 |
|--------|------|------|
| 0-11 | 12 | 序列号 (0-4095) |
| 12-21 | 10 | 机器 ID |
| 22-23 | 2 | 数据中心 ID |
| 24-63 | 40 | 时间戳 (毫秒级) |

### 11.3 Big Endian 转换

```rust
fn row_id_to_key(&self, row_id: u64) -> Vec<u8> {
    row_id.to_be_bytes().to_vec()
}

fn key_to_row_id(&self, key: &[u8]) -> Result<u64, ...> {
    if key.len() != 8 {
        return Err("Invalid row key length".into());
    }
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(key);
    Ok(u64::from_be_bytes(bytes))
}
```

### 11.4 前缀过滤优势

由于使用大端字节序存储，具有以下优势：

1. **时间范围扫描**：相同时间戳前缀的行在 RocksDB 中连续存储
2. **范围查询优化**：可以利用 RocksDB 的前缀迭代器进行高效扫描
3. **ID 单调性保证**：Snowflake ID 保证单调递增，写入顺序即存储顺序

### 11.5 依赖配置

**位置**: [multi_table_rocksdb/Cargo.toml](multi_table_rocksdb/Cargo.toml)

```toml
snowflake_me = { version = "0.5", features = ["ip-fallback"] }
```

---

## 12. 全文索引引擎设计

### 12.1 架构概述

全文索引引擎基于 **Tantivy 0.26** 实现，提供高性能的全文搜索能力。

```
┌─────────────────────────────────────────────────────────────────┐
│                    LaoflchDBServer                            │
│                           │                                   │
│                           ▼                                   │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │              Access 层                                   │  │
│  │  ┌────────────────────────────────────────────────────┐ │  │
│  │  │          IndexService (gRPC/REST)                 │ │  │
│  │  │  - CreateIndex / DropIndex / ListIndices          │ │  │
│  │  │  - AddDocument / GetDocument / DeleteDocument     │ │  │
│  │  │  - SearchIndex                                    │ │  │
│  │  └────────────────────────────────────────────────────┘ │  │
│  └───────────────────────────┬──────────────────────────────┘  │
│                              ▼                                 │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │              Service 层                                  │  │
│  │  ┌────────────────────────────────────────────────────┐ │  │
│  │  │            IndexServiceImpl                        │ │  │
│  │  │  - 索引元数据管理                                  │ │  │
│  │  │  - 认证检查                                        │ │  │
│  │  │  - 请求转发                                        │ │  │
│  │  └────────────────────────────────────────────────────┘ │  │
│  └───────────────────────────┬──────────────────────────────┘  │
│                              ▼                                 │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │           StorageEngine 层                               │  │
│  │  ┌────────────────────────────────────────────────────┐ │  │
│  │  │      TantivyStorageEngine                          │ │  │
│  │  │  ┌─────────┐ ┌─────────┐ ┌─────────┐           │ │  │
│  │  │  │ Index 1 │ │ Index 2 │ │ Index N │ ...       │ │  │
│  │  │  └────┬────┘ └────┬────┘ └────┬────┘           │ │  │
│  │  │       │           │           │                 │ │  │
│  │  │       └───────────┴───────────┘                 │ │  │
│  │  │           ↓                                      │ │  │
│  │  │  ┌──────────────────────────────────────────┐    │ │  │
│  │  │  │           Snowflake ID Generator         │    │ │  │
│  │  │  │  - 分布式唯一ID生成                      │    │ │  │
│  │  │  │  - 线程安全 (Mutex)                      │    │ │  │
│  │  │  └──────────────────────────────────────────┘    │ │  │
│  │  └────────────────────────────────────────────────────┘ │  │
│  └──────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

### 12.2 StorageEngine Trait

**位置**: [laoflchdb_engines/src/lib.rs](laoflchdb_engines/src/lib.rs)

```rust
#[async_trait::async_trait]
pub trait StorageEngine: Send + Sync + 'static {
    async fn create_index(&mut self, index_name: &str, fields: &[IndexField]) -> Result<u64, Box<dyn Error + Send + Sync>>;
    async fn drop_index(&mut self, index_name: &str) -> Result<(), Box<dyn Error + Send + Sync>>;
    async fn list_indices(&self) -> Result<Vec<String>, Box<dyn Error + Send + Sync>>;
    async fn get_index_fields(&self, index_name: &str) -> Result<Vec<String>, Box<dyn Error + Send + Sync>>;
    async fn get_index_meta(&self, index_name: &str) -> Result<Option<IndexMeta>, Box<dyn Error + Send + Sync>>;
    async fn get_index_stats(&self) -> Result<IndexStats, Box<dyn Error + Send + Sync>>;
    
    async fn add_document(&mut self, index_name: &str, doc_id: Option<&str>, fields: &std::collections::HashMap<String, String>) -> Result<String, Box<dyn Error + Send + Sync>>;
    async fn get_document(&self, index_name: &str, doc_id: &str) -> Result<Option<Document>, Box<dyn Error + Send + Sync>>;
    async fn delete_document(&mut self, index_name: &str, doc_id: &str) -> Result<(), Box<dyn Error + Send + Sync>>;
    async fn search(&self, index_name: &str, query: &str, fields: &[&str], limit: usize, offset: usize) -> Result<SearchResult, Box<dyn Error + Send + Sync>>;
    
    async fn shutdown(&self) -> Result<(), Box<dyn Error + Send + Sync>>;
}
```

### 12.3 TantivyStorageEngine

**位置**: [laoflchdb_index_tantivy_engine/src/lib.rs](laoflchdb_index_tantivy_engine/src/lib.rs)

#### 核心结构

```rust
pub struct TantivyStorageEngine {
    indices: RwLock<HashMap<String, Arc<RwLock<TantivyIndex>>>>,
    index_path: String,
    snowflake: Mutex<Snowflake>,
}

struct TantivyIndex {
    index: Index,
    reader: Arc<IndexReader>,
    searcher: Searcher,
}
```

#### 设计要点

| 设计点 | 说明 |
|--------|------|
| **并发安全** | 使用 `RwLock` 保护索引集合，`Mutex` 保护 Snowflake ID 生成器 |
| **延迟加载** | 索引按需创建和打开 |
| **持久化** | 索引数据存储在 `index_path` 目录下 |
| **Snowflake ID** | 自动生成分布式唯一 ID，用户不提供 doc_id 时使用 |

### 12.4 索引 Schema 设计

每个索引对应一个 Tantivy Schema：

| 字段类型 | Tantivy 类型 | 说明 |
|---------|-------------|------|
| `TEXT` | `TextField` | 全文索引字段，支持分词 |
| `STRING` | `StrField` | 字符串字段，精确匹配 |
| `INT` | `I64Field` | 整数字段 |
| `FLOAT` | `F64Field` | 浮点数字段 |

### 12.5 文档 ID 处理

- **用户提供**: 如果用户在 `AddDocumentRequest` 中提供了 `doc_id`，直接使用该 ID
- **自动生成**: 如果用户未提供 `doc_id`，使用 Snowflake ID 生成器生成唯一 ID
- **存储方式**: doc_id 作为 Tantivy 文档的一个字段存储，便于后续检索

### 12.6 搜索流程

```
搜索请求 → IndexService → TantivyStorageEngine.search()
    → 创建 QueryParser → 解析查询字符串
    → 获取 Searcher → 执行搜索
    → 遍历结果 → 构建 SearchResult
    → 返回响应
```

---

## 13. 向量化服务设计 (VectorService)

### 13.1 架构概述

向量化服务基于 **Candle 0.10 + CUDA** 实现，支持文本模型 (BERT/XLM-RoBERTa) 和视觉模型 (ViT) 的向量化推理。

```
┌─────────────────────────────────────────────────────────────────┐
│                    LaoflchDBServer                            │
│                           │                                   │
│                           ▼                                   │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │              VectorService (gRPC)                        │  │
│  │  - CreateEmbedding / ComputeSimilarity                   │  │
│  │  - LoadModel / UnloadModel / ListModels                 │  │
│  │  - ListLoadableModels / GetModelInfo                    │  │
│  └────────────────────────┬─────────────────────────────────┘  │
│                           │                                    │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │              VectorServiceImpl (核心引擎)                 │  │
│  │  ┌─────────────────────┐ ┌───────────────────────────┐  │  │
│  │  │   RealBertModel     │ │   VisionTransformer       │  │  │
│  │  │  (文本模型推理)      │ │  (视觉模型推理)            │  │  │
│  │  │  - BERT/XLM-RoBERTa │ │  - ViT Patch Embedding    │  │  │
│  │  │  - Tokenizer 编码   │ │  - Multi-Head Attention   │  │  │
│  │  │  - Mean Pooling     │ │  - CLS Pooling            │  │  │
│  │  │  - L2 Normalization │ │  - L2 Normalization       │  │  │
│  │  └─────────────────────┘ └───────────────────────────┘  │  │
│  └──────────────────────────────────────────────────────────┘  │
│                           │                                    │
│                           ▼                                    │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │              Candle 推理引擎 (CPU/CUDA)                  │  │
│  │  - 模型权重加载 (SafeTensors)                            │  │
│  │  - CUDA 加速推理                                        │  │
│  │  - 自动模型类型检测 (文本/视觉)                           │  │
│  └──────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

### 13.2 支持的模型类型

| 模型类型 | 示例模型 | 架构 | 输入 | 维度 |
|---------|---------|------|------|------|
| 文本模型 | bge-small-zh-v1.5 | BERT | texts | 512 |
| 文本模型 | bge-m3 | XLM-RoBERTa | texts | 1024 |
| 视觉模型 | jina-clip-v2 | ViT-L/14 | images | 1024 |
| 视觉模型 | siglip2 | ViT-B/16 | images | 768 |

### 13.3 模型自动加载

启动时通过配置控制模型加载：

```yaml
vector_service:
  enabled: true
  auto_load: true
  load_models: ["bge-small-zh-v1.5", "bge-m3", "jina-clip-v2", "siglip2"]
```

- **文本模型**: 需要 `config.json` + `tokenizer.json` + `model.safetensors`
- **视觉模型**: 需要 `config.json` (含 `vision_config`) + `model.safetensors`，不需要 tokenizer

### 13.4 图片向量化流程

```
图片输入 (PNG/JPEG) → ImageProcessor 解码
    → resize 到模型指定尺寸 (224/512)
    → 归一化 (mean/std)
    → 转 Tensor
    → Patch Embedding (Conv2d)
    → 添加 CLS Token + Position Embedding
    → Transformer Encoder 推理
    → CLS 向量输出
    → L2 归一化
```

**位置**: [laoflchdb_vector_service/src/vision_encoder.rs](laoflchdb_vector_service/src/vision_encoder.rs)

---

## 14. 嵌入向量索引服务设计 (EmbeddingIndexService)

### 14.1 架构概述

嵌入向量索引服务基于 **HNSW (Hierarchical Navigable Small World)** 算法实现，提供高性能的近似最近邻 (ANN) 搜索。

```
┌─────────────────────────────────────────────────────────────────┐
│                    LaoflchDBServer                            │
│                           │                                   │
│                           ▼                                   │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │           EmbeddingIndexService (gRPC)                   │  │
│  │  - InsertEmbedding / SearchEmbedding / DeleteEmbedding   │  │
│  │  - GetIndexInfo / SaveSnapshot / LoadSnapshot           │  │
│  └────────────────────────┬─────────────────────────────────┘  │
│                           │                                    │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │           EmbeddingIndexServiceImpl                      │  │
│  │  ┌────────────────────────────────────────────────────┐  │  │
│  │  │           HnswIndex (HNSW 算法)                    │  │  │
│  │  │  - 分层图结构 (multi-layer graph)                  │  │  │
│  │  │  - 余弦距离度量                                    │  │  │
│  │  │  - 批量插入                                        │  │  │
│  │  │  - 范围搜索                                        │  │  │
│  │  └────────────────────────────────────────────────────┘  │  │
│  │  ┌────────────────────────────────────────────────────┐  │  │
│  │  │           KvStore (向量持久化)                      │  │  │
│  │  │  - RocksDB 存储向量 ID → 向量数据映射               │  │  │
│  │  │  - 支持向量数据的持久化和恢复                        │  │  │
│  │  └────────────────────────────────────────────────────┘  │  │
│  │  ┌────────────────────────────────────────────────────┐  │  │
│  │  │           Snapshot Manager (快照管理)               │  │  │
│  │  │  - HNSW 图拓扑快照保存和加载                       │  │  │
│  │  │  - 支持快速重启恢复                                │  │  │
│  │  └────────────────────────────────────────────────────┘  │  │
│  └──────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

### 14.2 配置参数

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

| 参数 | 说明 | 默认值 |
|------|------|--------|
| dim | 向量维度 | 512 |
| m | HNSW 图每个节点的最大连接数 | 32 |
| ef_construction | 图构建时的搜索宽度 | 200 |
| ef_search | 搜索时的搜索宽度 | 50 |
| max_elements | 索引最大容量 | 1000000 |

### 14.3 搜索流程

```
搜索请求 → SearchEmbedding
    → 余弦距离计算
    → HNSW 图遍历 (多层搜索)
    → 返回 Top-K 最近邻
    → 从 KvStore 加载向量数据 (可选)
    → 返回搜索结果
```

---

## 15. 核心依赖

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

---

## 16. 架构设计原则

### 16.1 可扩展性设计

- **接入层**：可以添加新的协议实现
- **引擎层**：可以添加新的存储引擎实现 (LevelDB、Memory DB 等)
- **服务层**：可以添加新的服务能力 (事务、索引等)
- **Schema 层**：可以轻松扩展新的 Schema，每个 Schema 独立管理

### 16.2 关注点分离

- **Access 层**：只负责协议接入，不关心业务逻辑
- **Service 层**：只关心业务逻辑，管理 Schema 生命周期
- **SchemaManager**：管理多个 Schema 的引擎实例
- **DBEngine 层**：只关心单个 Schema 的存储操作，不关心业务逻辑

---

## 17. 版本历史

### 0.1.4 (当前)
- **向量化服务 (VectorService)**: 基于 Candle 0.10 + CUDA 实现文本和图片向量化推理，支持 BERT/XLM-RoBERTa/ViT 模型
- **嵌入向量索引服务 (EmbeddingIndexService)**: 基于 HNSW 算法实现近似最近邻搜索，支持向量持久化和快照管理
- **跨 Schema JOIN 支持**: 完整的跨不同 Schema 之间的表 JOIN 操作
- **自定义 laoflchdb Catalog**: 使用 `TableReference::full("laoflchdb", schema, table)` 注册表
- **动态 Schema 注册**: SQL 查询时动态注册缺失的 Schema 到 DataFusion
- **多表 JOIN 支持**: 支持三表及以上的跨 Schema JOIN
- **JOIN 类型支持**: INNER JOIN、LEFT JOIN、RIGHT JOIN、FULL OUTER JOIN
- **测试覆盖**: 新增 cross_schema_join_tests.rs 和 test_cross_schema_join.py
- **Python 回归测试**: 新增 test_final.py 完整回归测试套件
- **Protobuf 修复**: 修复测试端 rpc.proto 与服务端不一致问题
- **优雅关闭功能**: 支持 SIGINT/SIGTERM 信号处理，自动刷新 RocksDB 数据并释放锁文件
- **shutdown 接口**: 在 `StorageEngine` 和 `DatabaseService` trait 中添加 `shutdown` 方法

### 0.1.3
- **lsql 命令行客户端**: 类似 PostgreSQL psql 的交互式 SQL 客户端，支持 gRPC 连接
- **版本支持**: `laoflchdb` 和 `lsql` 均支持 `--version` 选项，版本号从各自的 Cargo.toml 读取
- **ListSchemas API**: 新增 gRPC API 用于列出所有可用的 Schema
- **execute_query 日志**: 添加详细的 SQL 执行日志输出，便于调试和性能分析
- **错误处理优化**: SQL 执行错误时不退出进程，只打印错误信息
- **Schema 验证**: 切换和默认 Schema 时验证是否存在
- **幂等初始化**: `init` 命令支持幂等执行，`sys` Schema 存在则跳过，`example` Schema 存在则删除重建
- **user 表**: 初始化时自动在 `sys` Schema 下创建 `user` 表存储用户信息
- **CLI 优化**: 移除 help 子命令，增加 `--example` 参数详细说明
- **测试增强**: 新增 lsql_client_tests.rs、sql_advanced_tests.rs、init_idempotent_tests.rs、cli_tests.rs、test_grpc_sql_advanced.py、test_grpc_sql_join.py

### 0.1.2
- **SQL 查询下推优化**: 支持 Filter、Project、Limit 下推到存储层
- **自定义物理执行算子**: `RocksScanExec` 直接对接 RocksDB，替代 MemTable
- **逻辑表达式支持**: AND/OR 条件下推
- **数据类型正确返回**: INT64、STRING、FLOAT、BYTES
- **代码重构**: `RocksDBTable` 拆分为独立文件 `rocksdb_table.rs`
- **文档更新**: 更新 README.md 和 design.md
- **测试完善**: 新增 SQL 查询验证测试和 gRPC SQL 查询测试

### 0.1.1
- 支持存储格式改为 protobuf Field 对象
- 实现完整的数据类型映射（INT64、STRING、FLOAT、BYTES）
- SQL 查询返回正确的数据类型（整数、字符串、浮点数）
- 修复谓词下推的比较逻辑
- 更新单元测试和集成测试
- 添加 Python 自动回归测试

### 0.1.0
- 实现基础 SQL 查询引擎
- 添加 filter、project、limit 下推优化
- 实现自定义物理执行算子 RocksScanExec
- 支持 projected_columns 列投影下推
- 完善异步架构设计
- 实现前缀过滤与 Snowflake ID 设计

---

**文档版本**: v0.1.4  
**最后更新**: 2026-07-11  
**项目**: laoflchDB-rust
