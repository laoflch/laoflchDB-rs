# laoflchdb_sql_df_engine

基于 DataFusion 的 SQL 查询引擎包，为 laoflchDB 提供 SQL 查询能力。

## 概述

`laoflchdb_sql_df_engine` 是 laoflchDB 的 SQL 查询引擎，基于 Apache DataFusion 构建，支持将标准 SQL 转换为查询计划并执行，最终通过自定义物理执行算子直接访问 RocksDB 存储引擎。

## 功能特性

### 核心功能
- **SQL 解析与执行**：支持标准 SQL 查询
- **查询规划与优化**：使用 DataFusion 的优化器
- **Arrow 数据格式**：采用 Apache Arrow 列式存储
- **自定义物理执行算子**：直接对接 RocksDB，避免 MemTable 包装
- **查询下推优化**：
  - Filter 条件下推
  - Project 列投影下推
  - Limit 限制条数下推

### 类型支持
- `Int64`：整数类型
- `Utf8`：字符串类型
- `Float64`：浮点类型
- `Binary`：二进制类型

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

## 架构设计

### 查询执行流程

```
SQL 字符串
    ↓
DataFusion SQL 解析器
    ↓
查询计划生成
    ↓
查询优化
    ↓
自定义物理执行算子 RocksScanExec
    ↓
查询下推到存储引擎
    ↓
Arrow RecordBatch
    ↓
转换为 QueryResult (protobuf)
```

### 自定义物理执行算子

`RocksScanExec`（在 multi_table_rocksdb 包中实现）替代了 DataFusion 的默认 MemTable，直接与 RocksDB 存储引擎交互，实现了：
- 列投影下推
- 过滤条件下推
- Limit 下推
- 异步数据流式读取

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

## 依赖配置

### Cargo.toml

```toml
[dependencies]
laoflchdb_engines = { path = "../laoflchdb_engines" }
datafusion = "53.1.0"
arrow = "58.3.0"
arrow-schema = "58.3.0"
arrow-array = "58.3.0"
tokio = { version = "1.0", features = ["rt"] }
async-trait = "0.1"
protobuf = "3.7"
```

## 目录结构

```
laoflchdb_sql_df_engine/
├── src/
│   └── lib.rs              # 主模块：SQL 引擎实现
├── Cargo.toml              # 依赖配置
└── README.md               # 本文档
```

## 与其他包的关系

```
laoflchdb_sql_df_engine
│
├── 依赖：laoflchdb_engines
│   ├── SQLEngine Trait
│   ├── StorageEngine Trait
│   └── QueryResult (protobuf)
│
└── 被 multi_table_rocksdb 依赖
    ├── 实现 DataFusionStorageEngine Trait
    ├── 提供 RocksScanExec 自定义物理执行算子
    └── 实现 projected_columns 列投影下推
```

## 性能优化

### 1. Arrow 列式存储
- 高效的向量化计算
- 良好的 SIMD 支持

### 2. 查询下推
- Filter：谓词在存储层执行
- Project：只读取需要的列
- Limit：提前终止扫描

### 3. 异步架构
- 使用 `tokio::sync::RwLock` 而非 `std::sync::RwLock`
- 所有查询方法均为异步
- 支持并发查询

## 版本历史

### 0.1.0 (当前)
- 基于 DataFusion 53.1.0 和 Arrow 58.3.0
- 实现 SQL 查询引擎核心功能
- 支持自定义物理执行算子
- 实现查询下推优化
- 支持 Async/Await 异步架构

## License

laoflchDB-rust 项目的一部分。
