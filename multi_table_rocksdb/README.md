# multi_table_rocksdb

基于 RocksDB 的多表存储引擎实现，是 laoflchDB 的核心存储层。

---

## 概述

`multi_table_rocksdb` 是 `DBEngine` trait 的具体实现，提供以下核心能力：

- **多表支持**：每个表对应一个 RocksDB Column Family
- **前缀过滤**：基于 Snowflake ID 和 Big Endian 实现高效范围扫描
- **CNF 查询**：支持 Conjunctive Normal Form 表达式查询
- **异步接口**：全部操作基于 tokio 异步运行时

---

## 目录结构

```
multi_table_rocksdb/
├── src/
│   └── multi_table_rocksdb.rs  # 核心实现
├── proto/                      # protobuf 定义（从 db_engine crate 引用）
├── build.rs                    # 编译脚本
├── Cargo.toml                  # 依赖配置
└── README.md                   # 本文件
```

---

## 核心架构

### 1. MultiTableRocksDBEngine

```rust
pub struct MultiTableRocksDBEngine {
    db: DB,              // RocksDB 实例
    schema_name: String, // 所属 Schema 名称
    snowflake: Snowflake, // Snowflake ID 生成器
}
```

### 2. 表与 Column Family 映射

| 概念 | RocksDB 对应 | 说明 |
|------|-------------|------|
| Schema | 独立 RocksDB 实例 | 每个 Schema 对应一个 db_path |
| Table | Column Family | 每个表对应一个 CF |
| Row | Key-Value 对 | 行数据存储为 CF 中的一条记录 |

### 3. Row ID 生成机制

#### 3.1 Snowflake ID 结构

```
64-bit Snowflake ID:
┌─────────────────────────────────────────────────────────────┐
│ 40 bits: Timestamp (毫秒级) │ 2 bits: DataCenter │ 10 bits: MachineID │ 12 bits: Sequence │
└─────────────────────────────────────────────────────────────┘
```

#### 3.2 Big Endian 转换

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

#### 3.3 前缀过滤优势

| 特性 | 说明 |
|------|------|
| **保序性** | Big Endian 确保 ID 在 RocksDB 中按时间顺序排序 |
| **前缀扫描** | 时间戳作为高位前缀，支持高效时间范围查询 |
| **单调性** | Snowflake ID 保证单调递增 |

---

## 查询接口实现

### 1. Query 结构

```protobuf
message Query {
    repeated TableFilter table_filters = 1;  // AND 关系
    optional uint32 limit = 2;
    optional uint32 offset = 3;
}

message TableFilter {
    string table_name = 1;
    repeated ColumnFilter column_filters = 2; // AND 关系
}

message ColumnFilter {
    string column_name = 1;
    repeated ColumnFilterCondition conditions = 2; // OR 关系
}
```

### 2. CNF 表达式计算

```
Query = (TableFilter_1 AND TableFilter_2 AND ...)
TableFilter = (ColumnFilter_1 AND ColumnFilter_2 AND ...)
ColumnFilter = (Condition_1 OR Condition_2 OR ...)
```

### 3. 支持的过滤操作符

| 操作符 | 说明 | 支持类型 |
|--------|------|----------|
| `EQ` | 等于 | Int64, String, Float |
| `NEQ` | 不等于 | Int64, String, Float |
| `GT` | 大于 | Int64, Float |
| `GTE` | 大于等于 | Int64, Float |
| `LT` | 小于 | Int64, Float |
| `LTE` | 小于等于 | Int64, Float |
| `IN` | 包含于列表 | Int64, String |
| `NOT_IN` | 不包含于列表 | Int64, String |
| `IS_NULL` | 为空 | 所有类型 |
| `IS_NOT_NULL` | 不为空 | 所有类型 |

### 4. 查询执行流程

```
┌─────────────────────────────────────────────────────────────────┐
│                        Query 执行流程                           │
├─────────────────────────────────────────────────────────────────┤
│  1. 解析 Query 结构                                            │
│         ↓                                                      │
│  2. 遍历 TableFilter（AND）                                    │
│         ↓                                                      │
│  3. 获取表的 Column Family                                     │
│         ↓                                                      │
│  4. 扫描 CF 中的所有行（RocksDB Iterator）                      │
│         ↓                                                      │
│  5. 对每行应用 ColumnFilter（AND）                             │
│         ↓                                                      │
│  6. 对每列应用 Condition（OR）                                  │
│         ↓                                                      │
│  7. 解码 Field 并执行比较                                      │
│         ↓                                                      │
│  8. 收集符合条件的行                                            │
│         ↓                                                      │
│  9. 应用 offset/limit                                          │
│         ↓                                                      │
│  10. 返回 QueryResult                                          │
└─────────────────────────────────────────────────────────────────┘
```

---

## 字段类型支持

### Field 编码

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
```

### 比较实现

```rust
fn compare_field_equals(
    &self, 
    field_bytes: &[u8], 
    field: &Field, 
    _column_type: ColumnType
) -> bool {
    let row_field = Field::decode(field_bytes)?;
    
    if let (Some(ref row_value), Some(ref value)) = (&row_field.value, &field.value) {
        match (row_value, value) {
            (Value::StringValue(s1), Value::StringValue(s2)) => s1.value == s2.value,
            (Value::IntegerValue(i1), Value::IntegerValue(i2)) => i1.value == i2.value,
            (Value::FloatValue(f1), Value::FloatValue(f2)) => f1.value == f2.value,
            _ => false,
        }
    } else {
        false
    }
}
```

---

## 元数据管理

### 1. 元数据 Key 格式

| 类型 | Key 格式 | 存储位置 |
|------|----------|----------|
| Schema 元数据 | `META-SCHEMA:{schema_name}` | default CF |
| 表元数据 | `META-TABLE:{table_name}:{table_id}` | default CF |
| 列元数据 | `META-COL:{table_id_fixed}:{col_name}:{col_id}:{col_type}` | default CF |

### 2. 自增 ID 管理

```rust
// Table ID: Schema 内唯一，从 0 开始递增
// Column ID: 表内唯一，从 0 开始递增

fn get_next_table_id(&self) -> Result<u64, ...> {
    // 从 SchemaMeta.next_auto_inc_table_id 获取并递增
}
```

---

## 依赖配置

### Cargo.toml

```toml
[dependencies]
rocksdb = { version = "0.50", features = ["snappy"] }
prost = "0.12"
async-trait = "0.1"
snowflake_me = { version = "0.5", features = ["ip-fallback"] }

laoflchdb_db_engine = { path = "../laoflchdb_db_engine" }
```

---

## 使用示例

### 创建引擎

```rust
use multi_table_rocksdb::MultiTableRocksDBEngine;
use laoflchdb_db_engine::{EngineOptions, DBEngine};

let options = EngineOptions {
    db_path: "./data/sys".to_string(),
    schema_name: "sys".to_string(),
};

let engine = MultiTableRocksDBEngine::new(&options)?;
```

### 创建表

```rust
engine.create_table("users", &[
    (0, "id", ColumnType::Int64),
    (1, "name", ColumnType::String),
]).await?;
```

### 添加行

```rust
let row = Row {
    row_type: 0,
    version: 1,
    data: vec![
        id_field.encode_to_vec(),
        name_field.encode_to_vec(),
    ],
};

let row_id = engine.add_row("users", &row).await?;
```

### 查询

```rust
use laoflchdb_db_engine::pb::{Query, TableFilter, ColumnFilter, ColumnFilterCondition, FilterOperator};

let query = Query {
    table_filters: vec![
        TableFilter {
            table_name: "users".to_string(),
            column_filters: vec![
                ColumnFilter {
                    column_name: "age".to_string(),
                    conditions: vec![
                        ColumnFilterCondition {
                            op: FilterOperator::Gte as i32,
                            value: Some(Field {
                                value: Some(Value::IntegerValue(Integer { value: 18 })),
                            }),
                            values: vec![],
                        },
                    ],
                },
            ],
        },
    ],
    limit: Some(10),
    offset: Some(0),
};

let result = engine.query(&query).await?;
```

---

## 测试覆盖

### 测试文件

**位置**: [tests/prefix_filter_tests.rs](../tests/prefix_filter_tests.rs)

### 测试用例

| 测试名称 | 测试内容 |
|---------|---------|
| `test_row_id_to_key_big_endian` | Big Endian 键转换 |
| `test_row_id_to_key_roundtrip` | ID 往返转换 |
| `test_big_endian_ordering_in_rocksdb` | RocksDB 中的排序 |
| `test_row_id_monotonic_increasing` | ID 单调递增 |
| `test_snowflake_id_distribution` | Snowflake ID 唯一性 |
| `test_query_with_cnf_filters` | CNF 查询功能 |
| `test_scan_rows_in_key_range` | 键范围扫描 |

### 运行测试

```bash
# 运行前缀过滤测试
cargo test --test prefix_filter_tests -- --test-threads=1
```

---

## 性能特性

### 前缀过滤优化

1. **时间范围查询**：相同时间戳前缀的行连续存储，支持高效范围扫描
2. **缓存友好**：时间相近的数据存储在相邻位置，提高缓存命中率
3. **写入顺序**：Snowflake ID 单调递增，写入顺序即存储顺序

### RocksDB 配置

```rust
let mut opts = Options::default();
opts.create_if_missing(true);
opts.create_missing_column_families(true);
// 可根据需求添加更多优化配置
```

---

## 设计模式

### 关注点分离

- **存储层**：只负责数据的读写，不关心业务逻辑
- **查询层**：只负责过滤逻辑，不关心存储细节
- **元数据层**：独立管理 Schema/Table/Column 元数据

### 接口抽象

通过 `DBEngine` trait 实现存储引擎的抽象，便于：
- 替换为其他存储引擎（如 LevelDB、Memory DB）
- 进行单元测试时使用 Mock 实现

---

## 版本历史

| 版本 | 变更 |
|------|------|
| 0.1.0 | 初始版本，支持基本 CRUD |
| 0.2.0 | 添加 Query 接口和 CNF 查询 |
| 0.3.0 | 引入 Snowflake ID 和前缀过滤 |

---

Copyright: laoflchDB-rust Project
