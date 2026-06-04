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
| **SQL 引擎** | [laoflchdb_db_engine/src/sql_engine.rs](laoflchdb_db_engine/src/sql_engine.rs) | DataFusion SQL 解析和执行引擎 |
| **DBEngine 接口** | [laoflchdb_db_engine/](laoflchdb_db_engine/) | 独立的数据库引擎接口定义 crate |
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
├── laoflchdb_db_engine/  # 独立的 DBEngine 接口定义 crate
│   ├── src/lib.rs
│   └── Cargo.toml
├── multi_table_rocksdb/  # 独立的 RocksDB 引擎实现 crate
│   ├── src/multi_table_rocksdb.rs
│   ├── build.rs         # 编译 RocksDB ldb 工具
│   └── Cargo.toml
├── tests/               # Rust 单元测试和集成测试
│   ├── basic_uuid_tests.rs
│   ├── protobuf_tests.rs
│   ├── rest_tests.rs
│   └── integration_tests.rs
├── tests_python/        # Python 自动化 E2E 测试
│   ├── test_e2e_grpc.py
│   └── test_e2e_rest.py
├── xtask/              # 构建和测试任务
│   ├── src/main.rs
│   └── Cargo.toml
├── run_tests.sh        # Rust 测试脚本
├── run_all_tests.sh    # 完整测试套件 (Rust + Python)
├── verify_tests.sh     # 快速验证测试
├── laoflchdb.yaml      # 配置文件示例
├── README.md
├── TESTING.md          # 测试指南
├── TEST_COVERAGE.md    # 测试覆盖报告
└── Cargo.toml
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

### DBEngine Trait (laoflchdb_db_engine crate)

**位置**: [laoflchdb_db_engine/src/lib.rs](laoflchdb_db_engine/src/lib.rs)

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
    
    // Query 接口 - 支持 CNF 查询
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
}
```

### DatabaseServiceImpl

```rust
pub struct DatabaseServiceImpl {
    schema_manager: Arc<SchemaManager>,
    default_schema: String,
}

impl DatabaseServiceImpl {
    pub async fn new(base_path: &str) -> Self {
        let schema_manager = SchemaManager::new(base_path).await;
        Self { 
            schema_manager: Arc::new(schema_manager),
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
    rpc Get (GetRequest) returns (GetResponse);
    rpc Put (PutRequest) returns (PutResponse);
    rpc Delete (DeleteRequest) returns (DeleteResponse);
    rpc CreateTable (CreateTableRequest) returns (CreateTableResponse);
    rpc ListTables (ListTablesRequest) returns (ListTablesResponse);
    rpc GetTableMeta (GetTableMetaRequest) returns (GetTableMetaResponse);
}
```

### REST API 接口

详细文档: [REST_API.md](REST_API.md)

| 端点 | 方法 | 说明 |
|------|------|------|
| `/health` | GET | 健康检查 |
| `/api/v1/tables` | POST | 创建表 |
| `/api/v1/schemas/{schema}/tables` | GET | 列出表 |
| `/api/v1/schemas/{schema}/tables/{table}` | GET | 获取表元数据 |
| `/api/v1/put` | POST | 插入数据 |
| `/api/v1/get` | GET | 读取数据 |
| `/api/v1/delete` | POST | 删除数据 |

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

## 10. 命令行 CLI 能力

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

### 3. 启动服务

```bash
./target/release/laoflchDB-rust -c laoflchdb.yaml start
```

服务将同时启动：
- **gRPC 服务**: http://127.0.0.1:19777
- **REST API**: http://127.0.0.1:8080

### 4. 作为库使用

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

    // 4. 插入数据 (异步调用)
    let key = b"key_001";
    let value = b"value_001";
    service.put("sys", "my_table", key, value).await.unwrap();

    // 5. 读取数据 (异步调用)
    let result = service.get("sys", "my_table", key).await.unwrap();
    println!("Read: {:?}", result);

    // 6. 删除数据 (异步调用)
    service.delete("sys", "my_table", key).await.unwrap();
}
```

### 5. 测试 API

#### 使用 gRPC 客户端

```bash
python3 tests_python/test_e2e_grpc.py
```

#### 使用 REST API

```bash
# 健康检查
curl http://127.0.0.1:8080/health

# 运行完整 REST 测试
python3 tests_python/test_e2e_rest.py
```

---

## 14. 异步改造记录

### 主要变更

项目已从同步调用架构完全改造为异步调用架构：

1. **DBEngine Trait** ([laoflchdb_db_engine/src/lib.rs](file:///workspace/rust_space/laoflchDB-rust/laoflchdb_db_engine/src/lib.rs))：
   - 所有方法改为 `async fn`
   - 添加 `#[async_trait::async_trait]` 宏

2. **SQLEngine Trait** ([laoflchdb_db_engine/src/lib.rs](file:///workspace/rust_space/laoflchDB-rust/laoflchdb_db_engine/src/lib.rs))：
   - 所有方法改为 `async fn`
   - 使用 `tokio::sync::RwLock` 替代 `std::sync::RwLock`
   - 添加 `#[async_trait::async_trait]` 宏

3. **DataFusionSQLEngine** ([laoflchdb_db_engine/src/sql_engine.rs](file:///workspace/rust_space/laoflchDB-rust/laoflchdb_db_engine/src/sql_engine.rs))：
   - 实现 SQLEngine Trait
   - 使用 `tokio::sync::RwLock` 保护 StorageEngine
   - 直接使用 async/await 语法，移除 `block_on` 调用

4. **MultiTableRocksDBEngine** ([multi_table_rocksdb/src/multi_table_rocksdb.rs](file:///workspace/rust_space/laoflchDB-rust/multi_table_rocksdb/src/multi_table_rocksdb.rs))：
   - 实现异步 DBEngine Trait
   - 保持 `new()` 方法同步用于初始化
   - 新增同步内部方法用于初始化

5. **Service 层** ([src/service/mod.rs](file:///workspace/rust_space/laoflchDB-rust/src/service/mod.rs))：
   - DatabaseService Trait 全部异步化
   - SchemaManager 使用 `tokio::sync::Mutex` 替代 `std::sync::Mutex`
   - DatabaseServiceImpl 支持异步构造
   - 集成 SQLEngine，提供 `sql_query` 方法

6. **Server 层** ([src/server/mod.rs](file:///workspace/rust_space/laoflchDB-rust/src/server/mod.rs))：
   - 启动和初始化方法改为异步
   - 使用 `tokio::spawn` 并发启动服务
   - 持有 SQLEngine 实例

### 解决的核心问题

| 问题 | 解决方案 |
|------|---------|
| `std::sync::Mutex` 不能跨 await 点 | 使用 `tokio::sync::Mutex` |
| `std::sync::RwLock` 不能跨 await 点 | 使用 `tokio::sync::RwLock` |
| 同步上下文中启动异步运行时冲突 | 将整个架构改为完全异步 |
| `block_on` 性能和死锁风险 | 移除所有 `block_on`，直接使用 async/await |
| SQL 引擎嵌套运行时冲突 | SQLEngine Trait 改为异步方法，移除 `block_on` |
| `RwLockReadGuard` 不是 `Send` | 使用 `tokio::sync::RwLock` 替代 `std::sync::RwLock` |

### 验证结果

- ✅ 所有 Rust 单元测试通过
- ✅ 所有 Rust 集成测试通过
- ✅ Python REST E2E 测试 10 项全部通过
- ✅ Python gRPC E2E 测试全部通过
- ✅ SQL 引擎集成测试通过
- ✅ 权限测试 21 项全部通过
- ✅ Protobuf 测试全部通过
- ✅ 前缀过滤测试 16 项全部通过---

## 15. SQL 引擎设计

### 15.1 架构概述

SQL 引擎基于 **DataFusion + Arrow + RocksDB** 实现，提供 SQL 查询能力。架构采用双层接口设计：

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

### 15.2 SQLEngine Trait 定义

**位置**: [laoflchdb_engines/src/lib.rs](file:///workspace/rust_space/laoflchDB-rust/laoflchdb_engines/src/lib.rs)

```rust
#[async_trait::async_trait]
pub trait SQLEngine: Send + Sync + 'static {
    async fn execute_query(&self, sql: &str) -> Result<QueryResult, Box<dyn std::error::Error + Send + Sync>>;
    
    async fn register_table(&mut self, table_name: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    async fn refresh_tables(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}
```

**方法说明**：

| 方法 | 说明 |
|------|------|
| `execute_query` | 执行 SQL 查询，返回 QueryResult |
| `register_table` | 将表注册到 DataFusion 上下文 |
| `refresh_tables` | 刷新所有表到 DataFusion 上下文 |

### 15.3 DataFusionSQLEngine 实现

**位置**: [laoflchdb_db_engine/src/sql_engine.rs](laoflchdb_db_engine/src/sql_engine.rs)

```rust
pub struct DataFusionSQLEngine<E: StorageEngine> {
    storage_engine: Arc<tokio::sync::RwLock<E>>,
    ctx: SessionContext,
}
```

**核心组件**：

| 组件 | 类型 | 说明 |
|------|------|------|
| `storage_engine` | `Arc<tokio::sync::RwLock<E>>` | 存储引擎引用 |
| `ctx` | `SessionContext` | DataFusion 会话上下文 |

### 15.4 数据转换流程

#### 15.4.1 存储层到 Arrow 格式

```
RocksDB 存储                Arrow 格式
─────────────────────────────────────────
Row {                       RecordBatch
  data: Vec<Vec<u8>>   ──►   Schema + Arrays
}                            │
                             ▼
每个 Field bytes ──► 解析 ──► Arrow Array
```

**类型映射**：

| ColumnType | Arrow DataType |
|------------|----------------|
| COLUMN_TYPE_STRING | Utf8 |
| COLUMN_TYPE_INT64 | Int64 |
| COLUMN_TYPE_FLOAT | Float64 |
| COLUMN_TYPE_BYTES | Binary |

#### 15.4.2 Arrow 格式到查询结果

```rust
fn arrow_to_query_result(&self, schema: &Schema, batch: &RecordBatch) -> QueryResult {
    // 遍历 RecordBatch 的每一行
    for i in 0..batch.num_rows() {
        // 遍历每一列
        for (j, field) in schema.fields().iter().enumerate() {
            // 根据 Arrow 类型转换为 protobuf Field
            match field.data_type() {
                DataType::Utf8 => { /* String */ }
                DataType::Int64 => { /* Integer */ }
                DataType::Float64 => { /* Float */ }
                DataType::Binary => { /* Bytes */ }
            }
        }
    }
}
```

### 15.5 表注册流程

```rust
async fn register_table(&mut self, table_name: &str) -> Result<(), ...> {
    // 1. 从存储引擎获取表数据
    let engine = self.storage_engine.read().await;
    let columns = engine.list_table_cols(table_name).await?;
    let rows = engine.scan_table(table_name, None).await?;
    
    // 2. 转换为 Arrow 格式
    let schema = Schema::new(arrow_fields);
    let batch = RecordBatch::try_new(Arc::new(schema), merged_arrays)?;
    
    // 3. 创建内存表并注册到 DataFusion
    let table = MemTable::try_new(batch.schema().clone(), vec![vec![batch]])?;
    self.ctx.register_table(TableReference::bare(table_name), Arc::new(table))?;
    
    Ok(())
}
```

### 15.7 SQL 查询执行流程

```
SQL 字符串
    │
    ▼
┌─────────────────────┐
│ DataFusion SQL 解析  │
└─────────────────────┘
    │
    ▼
┌─────────────────────┐
│ 查询计划生成         │
└─────────────────────┘
    │
    ▼
┌─────────────────────┐
│ 查询优化             │
└─────────────────────┘
    │
    ▼
┌─────────────────────┐
│ 执行查询             │
└─────────────────────┘
    │
    ▼
┌─────────────────────┐
│ RecordBatch 结果     │
└─────────────────────┘
    │
    ▼
┌─────────────────────┐
│ 转换为 QueryResult   │
└─────────────────────┘
```

### 15.8 异步设计要点

**使用 `tokio::sync::RwLock` 而非 `std::sync::RwLock`**：

```rust
// 正确：异步锁，可以跨 await 点
let engine = self.storage_engine.read().await;
let columns = engine.list_table_cols(table_name).await?;

// 错误：同步锁，不能跨 await 点
let engine = self.storage_engine.read().unwrap();  // std::sync::RwLock
let columns = engine.list_table_cols(table_name).await?;  // 编译错误！
```

**原因**：
- `std::sync::RwLockReadGuard` 不是 `Send`，不能跨 await 点
- `tokio::sync::RwLock` 的锁是异步的，可以安全跨 await 点

### 15.9 与 Server 的集成

**位置**: [src/server/mod.rs](file:///workspace/rust_space/laoflchDB-rust/src/server/mod.rs)

```rust
pub struct LaoflchDBServer {
    schema_manager: Arc<SchemaManager>,
    sql_engine: Arc<tokio::sync::RwLock<DataFusionSQLEngine<MultiTableRocksDBEngine>>>,
    service: Arc<dyn DatabaseService>,
    // ...
}
```

**位置**: [src/service/mod.rs](file:///workspace/rust_space/laoflchDB-rust/src/service/mod.rs)

```rust
pub struct DatabaseServiceImpl {
    schema_manager: Arc<SchemaManager>,
    sql_engine: Arc<tokio::sync::RwLock<DataFusionSQLEngine<MultiTableRocksDBEngine>>>,
    default_schema: String,
}

#[async_trait::async_trait]
impl DatabaseService for DatabaseServiceImpl {
    async fn sql_query(&self, schema: &str, sql: &str) -> Result<QueryResult, ...> {
        let sql_engine = self.sql_engine.read().await;
        sql_engine.execute_query(sql).await
    }
    
    async fn create_table(&self, schema: &str, table: &str, columns: &[...]) -> Result<u64, ...> {
        // 创建表后自动注册到 SQL 引擎
        let table_id = engine.create_table(&table, &columns).await?;
        
        if schema == "sys" {
            let mut sql_engine = self.sql_engine.write().await;
            sql_engine.register_table(&table).await?;
        }
        
        Ok(table_id)
    }
}
```

### 15.10 依赖配置

**位置**: [laoflchdb_db_engine/Cargo.toml](file:///workspace/rust_space/laoflchDB-rust/laoflchdb_db_engine/Cargo.toml)

```toml
[dependencies]
datafusion = "53.1.0"
arrow = "58.3.0"
arrow-schema = "58.3.0"
arrow-array = "58.3.0"
tokio = { version = "1.0", features = ["rt"] }
async-trait = "0.1"
protobuf = "3.7"
```

**位置**: [multi_table_rocksdb/Cargo.toml](file:///workspace/rust_space/laoflchDB-rust/multi_table_rocksdb/Cargo.toml)

```toml
[dependencies]
datafusion = "53.1.0"
arrow = "58.3.0"
arrow-schema = "58.3.0"
arrow-array = "58.3.0"
```

### 15.11 性能考虑

| 优化点 | 说明 |
|--------|------|
| 内存表 | DataFusion MemTable 将数据加载到内存，查询时无需磁盘 IO |
| Arrow 列式存储 | 向量化计算，SIMD 优化 |
| 查询优化 | DataFusion 内置查询优化器 |
| 异步锁 | `tokio::sync::RwLock` 允许并发读取 |
| 接口分离 | `DataFusionStorageEngine` 只包含 SQL 引擎所需方法，避免不必要的依赖 |

---

## 16. Query 接口设计

### 16.1 接口定义

`Query` 接口定义了服务层提交到 DBEngine 的数据查询信息，支持 **CNF (Conjunctive Normal Form) 表达式** 查询。

**位置**: [laoflchdb_db_engine/proto/query.proto](laoflchdb_db_engine/proto/query.proto)

```protobuf
message Query {
    repeated TableFilter table_filters = 1;  // 多个表过滤器，AND 关系 (CNF)
    optional uint32 limit = 2;               // 返回结果数量限制
    optional uint32 offset = 3;              // 跳过的结果数量
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
}

message QueryRow {
    string table_name = 1;
    uint64 row_id = 2;
    optional Row row = 3;
}
```

### 16.2 CNF 表达式结构

Query 接口的逻辑关系符合 **CNF (Conjunctive Normal Form)** 表达式：

```
Query = (TableFilter_1 AND TableFilter_2 AND ...)
TableFilter = (ColumnFilter_1 AND ColumnFilter_2 AND ...)
ColumnFilter = (Condition_1 OR Condition_2 OR ...)
```

### 16.3 DBEngine Query 方法

```rust
#[async_trait::async_trait]
pub trait DBEngine: Send + Sync + 'static {
    // Query 接口 - 支持 CNF 查询
    async fn query(&self, query: &Query) -> Result<QueryResult, Box<dyn std::error::Error + Send + Sync>>;
    // ... 其他方法
}
```

### 16.4 实现逻辑

**位置**: [multi_table_rocksdb/src/multi_table_rocksdb.rs](multi_table_rocksdb/src/multi_table_rocksdb.rs)

查询实现流程：

1. **表扫描**：遍历指定表的所有行（使用 RocksDB 迭代器）
2. **列过滤**：对每一行检查是否满足所有 ColumnFilter 条件
3. **条件比较**：支持 Int64、String、Float 等类型的比较操作
4. **结果组装**：将符合条件的行封装为 QueryResult 返回

---

## 17. 前缀过滤与 Snowflake ID 设计

### 17.1 Row ID 生成规则

multi_table 的 `row_id` 生成规则为：

1. **Snowflake 算法生成唯一 ID**
2. **转换为大端字节序 (Big Endian) 存储**

**核心优势**：
- **RocksDB 保序**：大端字节序确保 ID 在 RocksDB 中按时间顺序排序
- **前缀过滤**：时间戳作为 ID 的高位前缀，支持高效的时间范围扫描

### 17.2 Snowflake ID 结构

Snowflake ID 是一个 64 位整数，结构如下：

| 位范围 | 位数 | 说明 |
|--------|------|------|
| 0-11 | 12 | 序列号 (0-4095) |
| 12-21 | 10 | 机器 ID |
| 22-23 | 2 | 数据中心 ID |
| 24-63 | 40 | 时间戳 (毫秒级) |

### 17.3 Big Endian 转换

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

### 17.4 前缀过滤优势

由于使用大端字节序存储，具有以下优势：

1. **时间范围扫描**：相同时间戳前缀的行在 RocksDB 中连续存储
2. **范围查询优化**：可以利用 RocksDB 的前缀迭代器进行高效扫描
3. **ID 单调性保证**：Snowflake ID 保证单调递增，写入顺序即存储顺序

### 17.5 依赖配置

**位置**: [multi_table_rocksdb/Cargo.toml](multi_table_rocksdb/Cargo.toml)

```toml
snowflake_me = { version = "0.5", features = ["ip-fallback"] }
```

---

## 18. 数据初始化模块

### 18.1 Init 子命令

数据初始化模块用于初始化数据库运行必要数据和样例数据：

```bash
# 初始化数据库
./target/release/laoflchDB-rust init

# 初始化并创建示例数据
./target/release/laoflchDB-rust init --example
```

### 18.2 幂等性设计

`init --example` 支持幂等执行：

1. **检查 Schema 是否存在**：已存在则跳过创建
2. **检查表是否存在**：已存在则跳过创建
3. **检查数据是否已插入**：已存在则跳过插入

**位置**: [src/cli/mod.rs](src/cli/mod.rs)

### 18.3 示例数据

执行 `init --example` 后会创建：

- **example Schema**：示例数据库
- **products 表**：产品信息表（id, name, price, stock）
- **样例数据**：5 条产品记录

---

## 19. 测试

### 19.1 测试套件

项目包含完整的测试套件，覆盖单元测试、集成测试和端到端测试：

| 测试类型 | 位置 | 说明 |
|---------|------|------|
| Rust 单元测试 | [tests/](tests/) | 基础功能和 API 测试 |
| 前缀过滤测试 | [tests/prefix_filter_tests.rs](tests/prefix_filter_tests.rs) | 前缀过滤、Snowflake ID、Big Endian 测试 |
| Python E2E 测试 | [tests_python/](tests_python/) | gRPC 和 REST 端到端测试 |

### 19.2 前缀过滤测试覆盖

| 测试名称 | 测试内容 |
|---------|---------|
| `test_row_id_to_key_big_endian` | 测试 row_id 到大端字节序键的转换 |
| `test_row_id_to_key_roundtrip` | 测试 row_id 的往返转换 |
| `test_big_endian_ordering_in_rocksdb` | 测试 RocksDB 中的大端字节序排序 |
| `test_row_id_monotonic_increasing` | 测试 row_id 的单调递增性 |
| `test_snowflake_id_distribution` | 测试 Snowflake ID 的唯一性分布 |
| `test_prefix_comparison_across_boundaries` | 测试边界处的前缀比较 |
| `test_query_with_cnf_filters` | 测试带 CNF 过滤器的查询功能 |
| `test_scan_rows_in_key_range` | 测试键范围内的行扫描 |

### 19.3 运行测试

```bash
# 运行所有 Rust 测试
./run_tests.sh

# 运行完整测试套件 (Rust + Python)
./run_all_tests.sh

# 快速验证
./verify_tests.sh

# 仅运行前缀过滤测试
cargo test --test prefix_filter_tests -- --test-threads=1
```

### 19.4 测试文档

- [TESTING.md](TESTING.md) - 测试指南和使用说明
- [TEST_COVERAGE.md](TEST_COVERAGE.md) - 测试覆盖报告
- [TEST_REPORT.md](TEST_REPORT.md) - 详细测试报告

### 运行测试

```bash
# 运行所有 Rust 测试
./run_tests.sh

# 运行完整测试套件 (Rust + Python)
./run_all_tests.sh

# 快速验证
./verify_tests.sh
```

### 测试文档

- [TESTING.md](TESTING.md) - 测试指南和使用说明
- [TEST_COVERAGE.md](TEST_COVERAGE.md) - 测试覆盖报告
- [TEST_REPORT.md](TEST_REPORT.md) - 详细测试报告

---

## 17. 异步调用设计

laoflchDB 分层架构之间通过 **async/await** 和 **tokio** 运行时实现完全异步调用。

### 核心技术

| 技术 | 说明 |
|------|------|
| `#[async_trait]` | 为 trait 提供 async fn 支持 |
| `#[tonic::async_trait]` | 为 gRPC 服务提供 async fn 支持 |
| `tokio::sync::Mutex` | 异步上下文中的线程安全锁 |
| `Arc<dyn Trait>` | 线程安全的共享所有权 |
| `Box<dyn Error + Send + Sync>` | 跨线程的错误传递 |
| `tokio` | 异步运行时 |
| `tokio::spawn` | 异步任务并发执行 |

### 异步 Mutex 锁设计

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

### 异步调用链路

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

### SQL 查询调用链路

```
SQL 查询请求
    ↓
Service 层 (DatabaseService.sql_query)
    ↓ .read().await (异步读锁)
SQLEngine 层 (DataFusionSQLEngine)
    ↓ .await (异步执行)
DataFusion SessionContext
    ↓ 读取已注册的内存表
RecordBatch 结果
    ↓
转换为 QueryResult
```

### 初始化同步与异步分离

MultiTableRocksDBEngine 的 `new()` 方法是同步的，用于初始化。在初始化完成后，所有操作都是异步的：

```rust
impl MultiTableRocksDBEngine {
    // 同步初始化 - 在 new() 中调用
    fn init_default_user_table(&mut self) -> Result<(), ...> {
        // 同步创建默认表
    }
    
    // 异步操作 - 通过 trait 接口调用
    async fn create_table(&mut self, table: &str, columns: &[...]) -> Result<u64, ...> {
        // 异步创建表逻辑
    }
}
```

---

## 可扩展性设计

- **接入层**：可以添加新的协议实现
- **引擎层**：可以添加新的存储引擎实现 (LevelDB、Memory DB 等)
- **服务层**：可以添加新的服务能力 (事务、索引等)
- **Schema 层**：可以轻松扩展新的 Schema，每个 Schema 独立管理

---

## 关注点分离

- **Access 层**：只负责协议接入，不关心业务逻辑
- **Service 层**：只关心业务逻辑，管理 Schema 生命周期
- **SchemaManager**：管理多个 Schema 的引擎实例
- **DBEngine 层**：只关心单个 Schema 的存储操作，不关心业务逻辑

---

## 架构升级说明

### 从单 crate 到多 crate

为了更好的代码组织和复用，项目已重构为多 crate 架构：

1. **laoflchdb_db_engine** - 独立的接口定义 crate
   - 定义 DBEngine trait
   - 定义 EngineOptions 结构体
   - 可以被多个引擎实现 crate 依赖

2. **multi_table_rocksdb** - 独立的 RocksDB 实现 crate
   - 实现 MultiTableRocksDBEngine
   - 包含 build.rs 用于编译 ldb 工具
   - 依赖 laoflchdb_db_engine crate

这种架构允许：
- 接口和实现分离
- 可以方便地添加新的引擎实现
- 更好的代码组织和复用

---

---

## 20. Docker 容器部署

### 20.1 生产环境部署

**配置文件**: [config/prod.yaml](config/prod.yaml)

| 服务 | 端口 |
|------|------|
| gRPC | 29777 |
| REST | 38080 |

### 20.2 部署命令

```bash
# 构建项目
cargo build --release

# 构建 Docker 镜像
cargo docker build

# 启动容器
cargo docker start

# 完整部署（构建 + 镜像 + 启动）
cargo docker deploy
```

### 20.3 Dockerfile

**生产镜像**: [Dockerfile.prod](Dockerfile.prod)

- 基于 Ubuntu 24.04
- 配置文件打包到镜像内部
- 数据目录挂载到宿主机

### 20.4 数据持久化

数据目录: `laoflch_db_data_prod/`

```bash
# 启动时挂载
docker run -d --name laoflchdb \
    -p 29777:29777 \
    -p 38080:38080 \
    -v $(pwd)/laoflch_db_data_prod:/app/data \
    laoflchdb-rust:prod
```

---

## 21. 自动回归测试

### 21.1 测试命令

```bash
# 测试本地环境（端口从 laoflchdb.yaml 读取）
cargo auto-test local

# 测试生产环境（端口从 config/prod.yaml 读取）
cargo auto-test prod
```

### 21.2 测试覆盖

| 测试类型 | 测试文件 | 用例数 |
|---------|---------|--------|
| REST API | [tests_python/test_e2e_rest.py](tests_python/test_e2e_rest.py) | 10 |
| gRPC API | [tests_python/test_final.py](tests_python/test_final.py) | 10 |

### 21.3 REST API 测试覆盖

| 测试项 | 说明 |
|--------|------|
| 健康检查 | 验证服务可用性 |
| 创建表 | CreateTable API |
| 列出表 | ListTables API |
| 获取表元数据 | GetTableMeta API |
| 插入数据 | Put API |
| 读取数据 | Get API |
| 更新数据 | Put (更新) API |
| 删除数据 | Delete API |
| 验证删除 | 确认数据已删除 |
| 错误处理 | 异常场景处理 |

### 21.4 gRPC API 测试覆盖

| 测试项 | 说明 |
|--------|------|
| 创建表 | CreateTable RPC |
| 列出表 | ListTables RPC |
| 获取表元数据 | GetTableMeta RPC |
| 插入数据 | Put RPC |
| 读取数据 | Get RPC |
| 更新数据 | Put (更新) RPC |
| 查询数据 | Query RPC (CNF 表达式) |
| 删除数据 | Delete RPC |
| 验证删除 | 确认数据已删除 |
| 错误处理 | 异常场景处理 |

---

## 22. API 文档

| 文档 | 说明 |
|------|------|
| [REST_API.md](REST_API.md) | REST API 完整文档 |
| [gRPC_API.md](gRPC_API.md) | gRPC API 完整文档 |

### 22.1 REST API 端点

| 端点 | 方法 | 说明 |
|------|------|------|
| `/health` | GET | 健康检查 |
| `/api/v1/tables` | POST | 创建表 |
| `/api/v1/schemas/{schema}/tables` | GET | 列出表 |
| `/api/v1/schemas/{schema}/tables/{table}` | GET/DELETE | 获取/删除表 |
| `/api/v1/put` | POST | 插入数据 |
| `/api/v1/get` | GET | 读取数据 |
| `/api/v1/delete` | POST | 删除数据 |
| `/api/v1/query` | POST | CNF 查询 |

### 22.2 gRPC 服务

| 方法 | 说明 |
|------|------|
| Get/Put/Delete | KV 操作 |
| CreateTable/DropTable/ListTables | 表管理 |
| AddRow/GetRow/UpdateRow/DeleteRow | 行操作 |
| Query | CNF 表达式查询 |

---

## 23. xtask 工具命令

| 命令 | 说明 |
|------|------|
| `cargo build` | 构建项目 (debug) |
| `cargo build --release` | 构建项目 (release) |
| `cargo docker build` | 构建 Docker 镜像 |
| `cargo docker deploy` | 完整部署 |
| `cargo docker start` | 启动容器 |
| `cargo auto-test local` | 本地环境测试 |
| `cargo auto-test prod` | 生产环境测试 |
| `cargo init` | 初始化数据库 |
| `cargo ldb` | 构建 ldb 工具 |
| `cargo all` | 构建所有 |

---

Copyright: laoflchDB-rust Project
