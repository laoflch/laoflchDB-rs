# laoflchDB-rust: Rust + RocksDB 封装 OLTP 数据库

基于 Rust + RocksDB 的单机 OLTP 数据库，完整 gRPC 服务接口支持，命令行启动独立运行。

---

## 编译环境要求

- GCC >= 13.2.0 (rust-rocksdb 0.50 C++20 兼容)
- Rust 1.75+

---

## 总体架构设计

laoflchDB 采用 **三层架构设计**，核心实体为 `LaoflchDBServer`，各层之间通过异步调用进行通信：

| 层次模块 | 位置 | 说明 |
|----------|------|------|
| **Server 层** | [src/server/mod.rs](src/server/mod.rs) | LaoflchDBServer 总入口，支持多协议启动 |
| **Access 层** | [src/access/mod.rs](src/access/mod.rs) | 接入服务层，负责协议接入和路由 |
| **Service 层** | [src/service/mod.rs](src/service/mod.rs) | 数据库基础服务能力 + SchemaManager |
| **DBEngine 层** | [src/db_engine/](src/db_engine/) | 数据库存储引擎抽象 |
| CLI 命令行 | [src/cli/mod.rs](src/cli/mod.rs) | clap 命令行参数解析 |
| 配置模块 | [src/config.rs](src/config.rs) | YAML 配置文件解析 |
| lib.rs | [src/lib.rs](src/lib.rs) | 库导出入口 |
| main.rs | [src/main.rs](src/main.rs) | 二进制 standalone 程序入口 |

**目录结构源码树：**
```
src/
├── access/          # 接入服务层: AccessService, GrpcService
│   ├── proto/      # gRPC RPC 服务接口定义
│   │   └── rpc.proto
│   └── mod.rs
├── server/          # Server 层: LaoflchDBServer 总入口
│   └── mod.rs
├── service/         # Service 层: DatabaseService + SchemaManager
│   └── mod.rs
├── db_engine/       # DBEngine 层: 存储引擎抽象
│   ├── mod.rs       # DBEngine trait 定义
│   └── engines/     # 具体引擎实现
│       ├── proto/   # 元数据、行、字段的 protobuf 定义
│       │   ├── field.proto
│       │   ├── metadata.proto
│       │   └── row.proto
│       ├── field.rs # Column 接口及实现类型
│       ├── row.rs   # Row 辅助函数
│       ├── mod.rs
│       └── rocksdb.rs # MultiTableRocksDBEngine 实现
├── cli/             # 命令行 Parser: start / init
│   └── mod.rs
├── config/          # YAML 配置解析
│   └── mod.rs
├── lib.rs           # 库模块 + proto 导出
└── main.rs          # 二进制入口
```

---

## 1. Schema 与 RocksDB 映射设计

### 核心映射关系

**每个 db_path 是一个 Schema，每个 Schema 是一个独立的 RocksDB 实例**：

```
db_path/ (根目录)
├── default/         # default Schema (RocksDB 实例)
│   ├── default      # default CF: 存储 db/table/col 元数据
│   ├── user         # user 表 CF: 存储 user 表数据
│   └── orders       # orders 表 CF: 存储 orders 表数据
│
├── analytics/       # analytics Schema (RocksDB 实例)
│   ├── default      # default CF: 存储 analytics 的元数据
│   ├── events       # events 表 CF
│   └── metrics      # metrics 表 CF
│
└── warehouse/       # warehouse Schema (RocksDB 实例)
    ├── default      # default CF: 存储 warehouse 的元数据
    └── inventory    # inventory 表 CF
```

### Column Family 设计

| Column Family | 用途 |
|---------------|------|
| `default` | 存储 db、table、col 级别的元数据 |
| `{table_name}` | 存储对应表的数据 |

### 元数据 Key 格式

| 类型 | Key 格式 | 示例 |
|------|----------|------|
| 数据库元数据 | `META-DB` | `META-DB` |
| 表元数据 | `META-TABLE:{table_name}` | `META-TABLE:user` |
| 字段元数据 | `META-COL:{table_id}:{column_name}:{column_id}` | `META-COL:1000:user_id:0` |

### 自增 ID 设计

| ID 类型 | 起始值 | 存储位置 | 说明 |
|---------|--------|----------|------|
| `table_id` | 1000 | `DatabaseMeta.next_auto_inc_table_id` | Schema 内唯一，每创建表 +1 |
| `column_id` | 0 | `TableMeta.next_auto_inc_column_id` | 表内唯一，每添加字段 +1 |

---

## 2. LaoflchDBServer 总入口架构

`LaoflchDBServer` 是整个数据库的统一入口实体，负责组装和启动三层服务：

```
┌─────────────────────────────────────────────────────────────┐
│                     LaoflchDBServer                          │
│                                                             │
│  ┌──────────────────────────────────────────────────────┐  │
│  │                  Access 层 (接入层)                    │  │
│  │  ┌─────────────┐ ┌─────────────┐ ┌─────────────┐  │  │
│  │  │ GrpcService  │ │ HttpService  │ │ ThriftService│  │  │
│  │  │   (gRPC)     │ │   (HTTP)     │ │   (Thrift)   │  │  │
│  │  └─────────────┘ └─────────────┘ └─────────────┘  │  │
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
│  │  │  │   (default)  │ │(analytics)   │ │(warehouse)│ │ │  │
│  │  │  └─────────────┘ └─────────────┘ └───────────┘ │ │  │
│  │  └─────────────────────────────────────────────────┘ │  │
│  └───────────────────────┬──────────────────────────────┘  │
│                          ↓                                   │
│  ┌──────────────────────────────────────────────────────┐  │
│  │                  DBEngine 层 (引擎层)                  │  │
│  │  ┌─────────────────┐ ┌─────────────────┐            │  │
│  │  │MultiTableRocksDB│ │MultiTableRocksDB│ ...        │  │
│  │  │    (default)    │ │  (analytics)    │            │  │
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
    server.start(config).await?;
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

## 4. DBEngine 层 (引擎层)

### DBEngine Trait

```rust
pub trait DBEngine: Send + Sync + 'static {
    fn create_table(&self, table: &str) -> Result<(), ...>;
    fn drop_table(&self, table: &str) -> Result<(), ...>;
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

### MultiTableRocksDBEngine

`DBEngine` trait 的 RocksDB 实现，**一个实例对应一个 Schema 和一个 RocksDB DB 实例**：

[src/db_engine/engines/rocksdb.rs](src/db_engine/engines/rocksdb.rs)

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

- 负责 gRPC 访问的接入、注册和路由
- 对接 Service 层的服务接口
- 支持多种协议扩展 (gRPC、HTTP、Thrift 等)

### AccessService

```rust
pub struct AccessService {
    service: Arc<dyn DatabaseService>,
}

impl AccessService {
    pub fn new(service: Arc<dyn DatabaseService>) -> Self;
    pub fn get_grpc_service(&self) -> GrpcService;
}
```

### GrpcService

gRPC 协议的具体实现，实现 `LaoflchDb` trait：

```rust
#[derive(Clone)]
pub struct GrpcService {
    service: Arc<dyn DatabaseService>,
}

impl GrpcService {
    pub fn new(service: Arc<dyn DatabaseService>) -> Self;
    pub fn into_server(self) -> LaoflchDbServer<Self>;
}
```

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

### Column 实现

| 类型 | Struct | ColumnType | 说明 |
|------|--------|------------|------|
| String | `StringColumn(String)` | `String` | UTF-8 字符串 |
| Integer | `IntegerColumn(i64)` | `Int64` | 64 位整数 |
| Bytes | `BytesColumn(Vec<u8>)` | `Bytes` | 二进制数据 |
| Float | `FloatColumn(f64)` | `Float` | 64 位浮点数 |
| List | `ListColumn(Vec<Vec<u8>>)` | `List` | 字节数组列表 |
| Image | `ImageColumn { data, format }` | `Image` | 图片数据+格式 |

---

## 8. gRPC RPC 服务接口

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

message GetRequest {
    string schema = 1;
    string table = 2;
    bytes key = 3;
}
```

---

## 9. YAML 配置文件

配置文件 `laoflchdb.yaml`：

```yaml
# laoflchDB 配置文件
db_path: ./laoflch_db_data    # 数据库根目录 (包含多个 Schema)
addr: 127.0.0.1:50051        # 默认 gRPC 服务监听地址
log_level: info              # 日志级别
```

---

## 10. 命令行 CLI 能力

```
Commands:
  start      # 以 standalone daemon 方式启动 gRPC 监听服务
    Options:
      -a, --addr <ADDR>            gRPC bind address
      -d, --db-path <DB_PATH>      数据库根目录

  init       # 初始化数据库
```

启动示例：
```bash
# 编译
cargo build --release

# 初始化数据库
./target/release/laoflchDB-rust init

# 启动服务
./target/release/laoflchDB-rust start
```

---

## 11. 核心依赖

| Rust Crate | 版本 | 用途 |
|------------|------|------|
| rust-rocksdb | 0.50 | KV 存储 (RocksDB v11.1.1) |
| tonic | 0.11 | gRPC HTTP/2 async 服务 |
| prost | 0.12 | protobuf 编解码 |
| clap | 4.5 | 命令行参数 |
| tokio | 1.0 | async runtime |
| async-trait | 0.1 | async trait 支持 |
| serde | 1.0 | YAML 配置序列化 |
| serde_yaml | 0.9 | YAML 解析 |

---

## 12. 快速开始

```bash
# 1. 编译
cargo build --release

# 2. 初始化数据库
./target/release/laoflchDB-rust init

# 3. 启动服务
./target/release/laoflchDB-rust start

# 4. 使用 gRPC 客户端访问
python3 tests_python/test_e2e_grpc.py
```

---

## 13. 异步调用设计

laoflchDB 三层架构之间通过 **async/await** 和 **tokio** 运行时实现异步调用。

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
gRPC 请求
    ↓
Access 层 (GrpcService)
    ↓ .await (异步调用)
Service 层 (DatabaseService)
    ↓ (SchemaManager 获取引擎)
DBEngine 层 (MultiTableRocksDBEngine)
    ↓
RocksDB 存储
```

---

## 可扩展性设计

- **接入层**：可以添加新的协议实现 (HTTP、Thrift 等)
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

Copyright: laoflchDB-rust Project
