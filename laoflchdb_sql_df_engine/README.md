# laoflchdb_sql_df_engine

基于 **Apache DataFusion** 的 SQL 查询引擎包，为 laoflchDB 提供高效的 SQL 查询能力。

---

## 概述

`laoflchdb_sql_df_engine` 是 laoflchDB 的 SQL 查询引擎，基于 **DataFusion 53.1.0** 和 **Arrow 58.3.0** 构建，支持将标准 SQL 转换为查询计划并执行，最终通过自定义物理执行算子直接访问 RocksDB 存储引擎。

---

## 功能特性

### 核心功能
- **SQL 解析与执行**：支持标准 SQL 查询
- **查询规划与优化**：使用 DataFusion 的优化器
- **Arrow 数据格式**：采用 Apache Arrow 列式存储
- **自定义物理执行算子**：`RocksScanExec` 直接对接 RocksDB，避免 MemTable 包装
- **查询下推优化**：
  - **Filter 条件下推**（支持 `=`, `!=`, `<`, `>`, `<=`, `>=`, `AND`, `OR`）
  - **Project 列投影下推**（只扫描需要的列）
  - **Limit 限制条数下推**（提前终止扫描）
- **完整数据类型支持**：INT64、STRING、FLOAT、BYTES
- **正确的数据类型返回**：SQL 查询返回正确的 JSON 数据类型（整数、字符串、浮点数）

### 类型支持
| 存储类型 | Arrow 类型 | JSON 返回类型 |
|----------|-----------|--------------|
| `INT64` | Int64 | 整数 (Number) |
| `STRING` | Utf8 | 字符串 (String) |
| `FLOAT` | Float64 | 浮点数 (Number) |
| `BYTES` | Binary | 字符串 (String, UTF-8 解码) |

---

## 核心结构

### DataFusionStorageEngine Trait

定义了存储引擎需要为 SQL 查询提供的接口：

```rust
#[async_trait::async_trait]
pub trait DataFusionStorageEngine: Send + Sync + 'static {
    async fn table_to_arrow(&self, table_name: &str) -> Result<(Schema, Vec<ArrayRef>, Vec<(i32, String)>), ...>;
    fn create_table_provider(&self, table_name: &str) -> Arc<dyn TableProvider>;
}
```

### DataFusionSQLEngine 结构体

主 SQL 引擎实现：

```rust
pub struct DataFusionSQLEngine<E: StorageEngine + DataFusionStorageEngine> {
    storage_engine: Arc<tokio::sync::RwLock<E>>,
    ctx: SessionContext,
}
```

### SQLEngine Trait 实现

实现了完整的 SQL 引擎接口：

```rust
#[async_trait::async_trait]
impl<E: StorageEngine + DataFusionStorageEngine + 'static> SQLEngine for DataFusionSQLEngine<E> {
    async fn execute_query(&self, sql: &str) -> Result<QueryResult, ...>;
    async fn register_table(&mut self, table_name: &str) -> Result<(), ...>;
    async fn refresh_tables(&mut self) -> Result<(), ...>;
}
```

---

## 架构设计

### 查询执行流程

```
SQL 字符串
    ↓
DataFusion SQL 解析器
    ↓
查询计划生成
    ↓
查询优化（CNF、谓词/投影/Limit下推）
    ↓
TableProvider.scan() 生成 RocksScanExec
    ↓
RocksScanExec 执行（调用 table_to_arrow_with_pushdown）
    ↓
存储引擎直接扫描（应用过滤、投影、限制）
    ↓
Arrow RecordBatch
    ↓
转换为 QueryResult (protobuf)
    ↓
REST API 返回 JSON（正确的数据类型）
```

### 自定义物理执行算子

`RocksScanExec`（在 `multi_table_rocksdb` 包中实现）替代了 DataFusion 的默认 MemTable，直接与 RocksDB 存储引擎交互：

| 优化项 | 说明 |
|--------|------|
| 列投影下推 | 只扫描需要的列，减少 IO |
| 过滤条件下推 | 在存储层执行谓词过滤，减少数据加载 |
| Limit 下推 | 提前终止扫描，减少数据传输 |
| 异步流式读取 | 使用 futures Stream 处理大数据集 |

### 谓词下推支持

`supports_filters_pushdown` 方法支持以下操作符的下推：

| 操作符类型 | 支持的操作符 |
|-----------|-------------|
| 比较操作符 | `=`, `!=`, `<`, `>`, `<=`, `>=` |
| 逻辑操作符 | `AND`, `OR`（所有子表达式都支持时） |

### 存储格式

数据存储使用 protobuf Field 对象格式：

```protobuf
message Field {
    oneof value {
        StringValue string_value = 1;
        IntegerValue integer_value = 2;
        FloatValue float_value = 3;
        BytesValue bytes_value = 4;
    }
}

message Row {
    RowType row_type = 1;
    int32 version = 2;
    repeated bytes data = 3;  // 每个元素是序列化的 Field 对象
}
```

---

## 使用方法

### 1. 创建引擎实例

```rust
use laoflchdb_sql_df_engine::DataFusionSQLEngine;
use std::sync::Arc;
use tokio::sync::RwLock;

// storage_engine 需同时实现 StorageEngine 和 DataFusionStorageEngine
let storage_engine = Arc::new(RwLock::new(MyStorageEngine::new()));
let sql_engine = DataFusionSQLEngine::new(storage_engine);
```

### 2. 注册表

```rust
// 注册单个表
sql_engine.register_table("users").await?;

// 刷新所有表
sql_engine.refresh_tables().await?;
```

### 3. 执行查询

```rust
let result = sql_engine.execute_query("SELECT id, name FROM users WHERE id > 100").await?;
```

---

## REST API 使用示例

### 创建表

```bash
curl -X POST http://localhost:8080/api/v1/tables \
  -H "Content-Type: application/json" \
  -d '{
    "schema": "sys",
    "table_name": "users",
    "columns": [
      {"name": "id", "column_type": "INT64"},
      {"name": "name", "column_type": "STRING"},
      {"name": "age", "column_type": "INT64"}
    ]
  }'
```

### 插入数据

```bash
curl -X POST http://localhost:8080/api/v1/schemas/sys/tables/users/rows \
  -H "Content-Type: application/json" \
  -d '{"row": {"row_type": 0, "version": 1, "data": ["1", "Alice", "30"]}}'
```

### SQL 查询

```bash
curl -X POST http://localhost:8080/api/v1/sql_query \
  -H "Content-Type: application/json" \
  -d '{"sql": "SELECT id, name, age FROM users WHERE age > 25"}'
```

### 查询结果

```json
{
    "success": true,
    "data": {
        "columns": ["id", "name", "age"],
        "rows": [[1, "Alice", 30], [2, "Bob", 28]]
    }
}
```

---

## 依赖配置

### Cargo.toml

```toml
[package]
name = "laoflchdb_sql_df_engine"
version = "0.1.2"
edition = "2021"

[dependencies]
laoflchdb_engines = { path = "../laoflchdb_engines" }
datafusion = "53.1.0"
arrow = "58.3.0"
arrow-schema = "58.3.0"
arrow-array = "58.3.0"
tokio = { version = "1.0", features = ["rt"] }
async-trait = "0.1"
protobuf = "3.7"
serde = "1.0"
serde_json = "1.0"
futures = "0.3"
```

---

## 目录结构

```
laoflchdb_sql_df_engine/
├── src/
│   └── lib.rs              # 主模块：SQL 引擎实现
├── Cargo.toml              # 依赖配置
└── README.md               # 本文档
```

---

## 与其他包的关系

```
laoflchdb_sql_df_engine
│
├── 依赖：laoflchdb_engines
│   ├── SQLEngine Trait
│   ├── StorageEngine Trait
│   ├── QueryResult (protobuf)
│   └── Field (protobuf)
│
└── 被 multi_table_rocksdb 依赖
    ├── 实现 DataFusionStorageEngine Trait
    ├── 提供 RocksScanExec 自定义物理执行算子
    ├── 实现 table_to_arrow_with_pushdown 方法
    └── 支持 protobuf Field 对象存储
```

---

## 性能优化

### 1. Arrow 列式存储
- 高效的向量化计算
- 良好的 SIMD 支持

### 2. 查询下推
- **Filter**：谓词在存储层执行，减少数据加载
- **Project**：只读取需要的列，减少 IO
- **Limit**：提前终止扫描，减少数据传输

### 3. 异步架构
- 使用 `tokio::sync::RwLock` 而非 `std::sync::RwLock`
- 所有查询方法均为异步
- 支持并发查询

### 4. protobuf 序列化
- 使用 protobuf 存储 Field 对象
- 高效的序列化/反序列化
- 支持多种数据类型

---

## 测试

项目包含完整的测试套件：

### Rust 测试
- 单元测试：`tests/*.rs`
- 集成测试：`tests/integration_tests.rs`

### Python 测试
- E2E 测试：`tests_python/test_e2e_rest.py`
- 回归测试：`tests_python/test_sql_query_validation.py`
- gRPC 测试：`tests_python/test_grpc_sql_query.py`

---

## 版本历史

### 0.1.2 (当前)
- **SQL 查询下推优化**: 支持 Filter、Project、Limit 下推到存储层
- **自定义物理执行算子**: `RocksScanExec` 直接对接 RocksDB
- **逻辑表达式支持**: AND/OR 条件下推
- **数据类型正确返回**: INT64、STRING、FLOAT、BYTES
- **代码重构**: `RocksDBTable` 拆分为独立文件
- **文档更新**: 更新 README.md

### 0.1.1
- 支持存储格式改为 protobuf Field 对象
- 实现完整的数据类型映射（INT64、STRING、FLOAT、BYTES）
- SQL 查询返回正确的数据类型（整数、字符串、浮点数）
- 修复谓词下推的比较逻辑

### 0.1.0
- 基于 DataFusion 53.1.0 和 Arrow 58.3.0
- 实现 SQL 查询引擎核心功能
- 支持自定义物理执行算子
- 实现查询下推优化
- 支持 Async/Await 异步架构

---

## License

laoflchDB-rust 项目的一部分。
