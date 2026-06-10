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
│   └── integration_tests.rs
├── tests_python/        # Python 自动化 E2E 测试
│   ├── test_e2e_grpc.py
│   ├── test_e2e_rest.py
│   ├── test_sql_query_validation.py    # SQL 查询验证测试
│   ├── test_grpc_sql_query.py          # gRPC SQL 查询测试
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

### 0.1.4 (当前)
- **表和字段注释支持**: 在 `TableMeta` 和 `ColumnMeta` 中添加了 `comment` 字段，支持语义化注释
- **订单交易系统示例**: `--example` 初始化时创建完整的订单交易系统表结构，包含表和字段注释
- **lsql 命令行客户端**: 类似 PostgreSQL psql 的交互式 SQL 客户端，支持 `\v` 作为 `\version` 的别名
- **ListSchemas API**: 新增 gRPC API 用于列出所有 Schema
- **execute_query 日志**: 添加详细的 SQL 执行日志输出
- **错误处理优化**: SQL 执行错误时不退出进程
- **Schema 验证**: 切换和默认 Schema 时验证是否存在

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
