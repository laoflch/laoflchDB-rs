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

async fn start_server(config: &DatabaseConfig, db_path: &str) {
    // 1. 创建 SchemaManager 管理多个 Schema (在 service 层)
    let schema_manager = Arc::new(
        SchemaManager::new(db_path)
    );

    // 2. 创建 Service 层
    let service_layer: Arc<dyn DatabaseService> = Arc::new(
        DatabaseServiceImpl::new(Arc::clone(&schema_manager))
    );

    // 3. 创建 Access 层
    let access_service = Arc::new(AccessService::new(Arc::clone(&service_layer)));

    // 4. 创建并启动 LaoflchDBServer
    let server = LaoflchDBServer::new(schema_manager, service_layer, access_service);
    server.start(config)?;
}
```

---

## 3. SchemaManager 设计 (Service 层)

`SchemaManager` 在 **Service 层** 实现，负责管理多个 Schema (RocksDB 实例)：

```rust
pub struct SchemaManager {
    engines: RwLock<HashMap<String, Arc<MultiTableRocksDBEngine>>>,
    base_path: String,
}

impl SchemaManager {
    pub fn new(base_path: &str) -> Self;
    
    pub fn get_schema_engine(&self, schema: &str) 
        -> Result<Arc<MultiTableRocksDBEngine>, Box<dyn std::error::Error + Send + Sync>>;
    
    pub fn list_schemas(&self) -> Vec<String>;
    
    pub fn create_schema(&self, schema: &str) 
        -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    pub fn drop_schema(&self, schema: &str) 
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

```rust
pub trait DBEngine: Send + Sync + 'static {
    fn create_table(&mut self, table: &str) -> Result<(), ...>;
    fn drop_table(&mut self, table: &str) -> Result<(), ...>;
    fn list_tables(&self) -> Result<Vec<String>, ...>;
    
    fn put(&self, table: &str, key: &[u8], value: &[u8]) -> Result<(), ...>;
    fn get(&self, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, ...>;
    fn delete(&self, table: &str, key: &[u8]) -> Result<(), ...>;
    
    fn put_meta(&self, key: &[u8], value: &[u8]) -> Result<(), ...>;
    fn get_meta(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ...>;
    fn scan_meta_prefix(&self, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>, ...>;
    fn delete_meta(&self, key: &[u8]) -> Result<(), ...>;
    
    fn get_schema_name(&self) -> &str;
}
```

### MultiTableRocksDBEngine (multi_table_rocksdb crate)

**位置**: [multi_table_rocksdb/src/multi_table_rocksdb.rs](multi_table_rocksdb/src/multi_table_rocksdb.rs)

`DBEngine` trait 的 RocksDB 实现，**一个实例对应一个 Schema 和一个 RocksDB DB 实例**：

```rust
pub struct MultiTableRocksDBEngine {
    db: DB,              // 一个 RocksDB DB 实例
    schema_name: String, // 对应一个 Schema
}

impl MultiTableRocksDBEngine {
    pub fn new(options: &EngineOptions) -> Result<Self, ...>;
    
    fn get_table_cf(&self, table: &str) -> String {
        table.to_string()  // 每个表对应一个 CF
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

```rust
pub trait DatabaseService: Send + Sync + 'static {
    fn init_database(&self) -> Result<(), ...>;
    fn create_schema(&self, schema: &str) -> Result<(), ...>;
    fn list_schemas(&self) -> Result<Vec<String>, ...>;
    fn drop_schema(&self, schema: &str) -> Result<(), ...>;
    
    fn put(&self, schema: &str, table: &str, key: &[u8], value: &[u8]) -> Result<(), ...>;
    fn get(&self, schema: &str, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, ...>;
    fn delete(&self, schema: &str, table: &str, key: &[u8]) -> Result<(), ...>;
    fn create_table(&self, schema: &str, table: &str, columns: &[...]) -> Result<u64, ...>;
    fn list_tables(&self, schema: &str) -> Result<Vec<String>, ...>;
    fn get_table_meta(&self, schema: &str, table: &str) -> Result<Option<TableMeta>, ...>;
}
```

### DatabaseServiceImpl

```rust
pub struct DatabaseServiceImpl {
    schema_manager: Arc<SchemaManager>,
    default_schema: String,
}

impl DatabaseServiceImpl {
    pub fn new(schema_manager: Arc<SchemaManager>) -> Self;
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

# 访问协议配置
access_protocols:
  - protocol: grpc
    enabled: true
    addr: 127.0.0.1:19777

  - protocol: rest
    enabled: true
    addr: 127.0.0.1:8080
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
| clap | 4.5 | 命令行参数 |
| tokio | 1.0 | async runtime |
| serde | 1.0 | YAML 配置序列化 |
| serde_yaml | 0.9 | YAML 解析 |

---

## 13. 快速开始

### 1. 编译

```bash
# 编译 Rust 代码
cargo build --release
```

### 2. 初始化数据库

```bash
./target/release/laoflchDB-rust -c laoflchdb.yaml init
```

### 3. 启动服务

```bash
./target/release/laoflchDB-rust -c laoflchdb.yaml start
```

服务将同时启动：
- **gRPC 服务**: http://127.0.0.1:19777
- **REST API**: http://127.0.0.1:8080

### 4. 测试 API

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

## 14. 测试

### 测试套件

项目包含完整的测试套件，覆盖单元测试、集成测试和端到端测试：

| 测试类型 | 位置 | 说明 |
|---------|------|------|
| Rust 单元测试 | [tests/](tests/) | 基础功能和 API 测试 |
| Python E2E 测试 | [tests_python/](tests_python/) | gRPC 和 REST 端到端测试 |

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

## 15. 异步调用设计

laoflchDB 分层架构之间通过 **async/await** 和 **tokio** 运行时实现异步调用。

### 核心技术

| 技术 | 说明 |
|------|------|
| `#[async_trait]` | 为 trait 提供 async fn 支持 |
| `#[tonic::async_trait]` | 为 gRPC 服务提供 async fn 支持 |
| `Arc<dyn Trait>` | 线程安全的共享所有权 |
| `Box<dyn Error + Send + Sync>` | 跨线程的错误传递 |
| `tokio` | 异步运行时 |

### 异步调用链路

```
gRPC/REST 请求
    ↓
Access 层 (GrpcService/RestService)
    ↓ .await (异步调用)
Service 层 (DatabaseService)
    ↓ (SchemaManager 获取引擎)
DBEngine 层 (MultiTableRocksDBEngine)
    ↓
RocksDB 存储
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

Copyright: laoflchDB-rust Project
