# 测试指南

## 快速开始

### 运行所有测试（包括Python自动化测试）
```bash
./run_all_tests.sh
```

### 仅运行Rust测试
```bash
./run_tests.sh
```

### 运行特定测试
```bash
# Rust 单元测试
cargo test --test basic_uuid_tests
cargo test --test protobuf_tests
cargo test --test rest_tests
cargo test --test integration_tests
cargo test --test sql_advanced_tests
cargo test --test init_idempotent_tests
cargo test --test cli_tests
cargo test --test cross_schema_join_tests  # 跨 Schema JOIN 测试
cargo test --test index_tests              # 全文索引测试

# Python 自动化测试
python3 tests_python/test_e2e_grpc.py
python3 tests_python/test_e2e_rest.py
python3 tests_python/test_grpc_sql_query.py
python3 tests_python/test_grpc_sql_advanced.py
python3 tests_python/test_grpc_sql_join.py
python3 tests_python/test_lsql_sql.py
python3 tests_python/test_list_schemas.py
python3 tests_python/test_data_validation.py
python3 tests_python/test_sql_query_validation.py
python3 tests_python/test_cross_schema_join.py  # 跨 Schema JOIN 测试
python3 tests_python/test_table_structure.py    # 表结构测试
python3 tests_python/test_final.py              # 完整回归测试
python3 tests_python/test_index_rest.py         # REST 全文索引测试
python3 tests_python/test_index_grpc.py         # gRPC 全文索引测试
```

---

## 测试类型

### 1. Rust 单元测试 (Unit Tests)

#### 基础UUID测试
- **文件**: `tests/basic_uuid_tests.rs`
- **数量**: 3个测试
- **内容**: 测试元数据键格式和常量定义

```bash
cargo test --test basic_uuid_tests
```

#### Protobuf测试
- **文件**: `tests/protobuf_tests.rs`
- **数量**: 2个测试
- **内容**: 测试Protobuf编解码功能

```bash
cargo test --test protobuf_tests
```

#### REST服务测试
- **文件**: `tests/rest_tests.rs`
- **数量**: 5个测试
- **内容**: 测试REST API端点功能

```bash
cargo test --test rest_tests
```

### 2. Rust 集成测试 (Integration Tests)

- **文件**: `tests/integration_tests.rs`
- **数量**: 3个测试
- **内容**: 测试完整的业务流程

```bash
cargo test --test integration_tests
```

### 3. Python 自动化测试 (E2E Tests)

#### gRPC 端到端测试
- **文件**: `tests_python/test_e2e_grpc.py`
- **内容**: 测试gRPC接口的数据写入和读取
- **测试流程**:
  1. 编译Rust release版本
  2. 启动gRPC服务
  3. 通过gRPC写入数据到user表
  4. 通过gRPC读取并验证数据

```bash
python3 tests_python/test_e2e_grpc.py
```

#### REST API 端到端测试
- **文件**: `tests_python/test_e2e_rest.py`
- **内容**: 测试REST API接口的完整CRUD流程（包含身份验证）
- **测试场景**:
  1. 健康检查
  2. 用户登录
  3. 未认证访问测试
  4. 创建表（需认证）
  5. 列出表（需认证）
  6. 获取表元数据（需认证）
  7. 插入数据（需认证）
  8. 读取数据（需认证）
  9. 更新数据（需认证）
  10. 删除数据（需认证）
  11. 验证删除（需认证）
  12. SQL查询（需认证）
  13. 错误处理
  14. 用户登出

```bash
python3 tests_python/test_e2e_rest.py
```

#### lsql 客户端 SQL 执行测试
- **文件**: `tests_python/test_lsql_sql.py`
- **内容**: 通过 lsql 命令行工具测试所有 SQL 功能
- **测试流程**:
  1. 通过 REST API 创建表和插入数据
  2. 使用 lsql 客户端工具执行 SQL 查询
  3. 验证查询结果正确
- **测试内容**:
  - 基本连接测试
  - SELECT 查询（全表、指定列、WHERE、ORDER BY、LIMIT）
  - 多表 JOIN（INNER JOIN、LEFT JOIN）
  - 查询系统表
  - 列别名
  - IN 条件查询

```bash
python3 tests_python/test_lsql_sql.py
```

#### 全文索引 REST API 测试
- **文件**: `tests_python/test_index_rest.py`
- **内容**: 测试全文索引 REST API 的完整功能
- **测试场景**:
  1. 创建全文索引
  2. 列出所有索引
  3. 获取索引字段信息
  4. 获取索引元数据
  5. 获取索引统计信息
  6. 创建包含多种字段类型的索引
  7. 添加文档（含自动生成 doc_id）
  8. 搜索文档（全文搜索）
  9. 多字段搜索
  10. 通过 doc_id 获取文档
  11. 删除文档
  12. 删除索引
  13. 未认证请求测试（应返回 403）
  14. 无效 Token 请求测试（应返回 403）
  15. 创建同名索引测试

```bash
python3 tests_python/test_index_rest.py
```

#### 全文索引 gRPC API 测试
- **文件**: `tests_python/test_index_grpc.py`
- **内容**: 测试全文索引 gRPC API 的完整功能
- **测试场景**:
  1. 创建全文索引
  2. 列出所有索引
  3. 获取索引字段信息
  4. 获取索引元数据
  5. 获取索引统计信息
  6. 创建包含多种字段类型的索引
  7. 添加文档
  8. 搜索文档（全文搜索）
  9. 多字段搜索
  10. 通过 doc_id 获取文档
  11. 删除文档
  12. 删除索引
  13. 未认证请求测试（应失败）
  14. 无效 Token 请求测试（应失败）
  15. 创建同名索引测试
  16. 添加多个文档并进行真实搜索测试

```bash
python3 tests_python/test_index_grpc.py
```

---

## 测试覆盖率

### 接口覆盖率

| 接口类型 | 接口 | 测试覆盖 |
|---------|------|---------|
| **gRPC** | Put | ✅ |
| **gRPC** | Get | ✅ |
| **gRPC** | Delete | ✅ |
| **gRPC** | CreateTable | ✅ |
| **gRPC** | CreateIndex | ✅ |
| **gRPC** | DropIndex | ✅ |
| **gRPC** | ListIndices | ✅ |
| **gRPC** | AddDocument | ✅ |
| **gRPC** | GetDocument | ✅ |
| **gRPC** | DeleteDocument | ✅ |
| **gRPC** | SearchIndex | ✅ |
| **REST** | `/health` | ✅ |
| **REST** | `/api/v1/tables` | ✅ |
| **REST** | `/api/v1/schemas/{schema}/tables` | ✅ |
| **REST** | `/api/v1/schemas/{schema}/tables/{table}` | ✅ |
| **REST** | `/api/v1/put` | ✅ |
| **REST** | `/api/v1/get` | ✅ |
| **REST** | `/api/v1/delete` | ✅ |
| **REST** | `/api/v1/index/indices` | ✅ |
| **REST** | `/api/v1/index/indices/{name}` | ✅ |
| **REST** | `/api/v1/index/indices/{name}/docs` | ✅ |
| **REST** | `/api/v1/index/indices/{name}/docs/{doc_id}` | ✅ |
| **REST** | `/api/v1/index/indices/{name}/search` | ✅ |

### 功能覆盖率

| 功能模块 | 测试覆盖 |
|---------|---------|
| 数据库初始化 | ✅ |
| Schema管理 | ✅ |
| 表创建 | ✅ |
| 表元数据 | ✅ |
| 数据CRUD | ✅ |
| 错误处理 | ✅ |
| 并发访问 | ✅ |
| 数据持久化 | ✅ |
| 用户认证 | ✅ |
| 令牌管理 | ✅ |
| 密码验证 | ✅ |
| 全文索引创建 | ✅ |
| 全文索引删除 | ✅ |
| 文档添加 | ✅ |
| 文档查询 | ✅ |
| 文档删除 | ✅ |
| 全文搜索 | ✅ |
| 多字段搜索 | ✅ |
| 自动ID生成 | ✅ |
| 认证拦截 | ✅ |

---

## 完整测试方案

### 运行完整测试套件

```bash
./run_all_tests.sh
```

这将执行以下步骤：

1. **Rust 单元测试**
   - 基础UUID测试 (3个)
   - Protobuf测试 (2个)
   - REST服务测试 (5个)
   - 集成测试 (3个)

2. **编译 Release 版本**
   - 编译Rust项目

3. **初始化数据库**
   - 清理旧数据
   - 初始化新数据库

4. **启动服务**
   - 启动gRPC服务 (127.0.0.1:19777)
   - 启动REST服务 (127.0.0.1:8080)

5. **Python 自动化测试**
   - gRPC 端到端测试
   - REST API 端到端测试

6. **清理**
   - 停止服务
   - 清理测试数据

### 测试统计

| 测试类型 | 测试数量 | 状态 |
|---------|---------|------|
| Rust CLI测试 | 6 | ✅ |
| Rust 单元测试 | 10 | ✅ |
| Rust SQL高级测试 | 6 | ✅ |
| Rust lsql客户端测试 | 4 | ✅ |
| Rust 幂等初始化测试 | 4 | ✅ |
| Rust 跨Schema JOIN测试 | 4 | ✅ |
| Rust 全文索引测试 | 4 | ✅ |
| Python gRPC测试 | 1 | ✅ |
| Python REST测试 | 10 | ✅ |
| Python SQL测试 | 3 | ✅ |
| Python 跨Schema JOIN测试 | 1 | ✅ |
| Python 表结构测试 | 1 | ✅ |
| Python 完整回归测试 | 1 | ✅ |
| Python 全文索引REST测试 | 1 | ✅ |
| Python 全文索引gRPC测试 | 1 | ✅ |
| **总计** | **57** | **✅** |

---

## 调试测试

### 显示Rust测试输出
```bash
cargo test -- --nocapture
```

### 运行单个Rust测试
```bash
cargo test test_rest_health -- --nocapture
```

### 查看Python测试详细日志
```bash
python3 tests_python/test_e2e_grpc.py
python3 tests_python/test_e2e_rest.py
```

### 查看服务日志
```bash
tail -f /tmp/server.log
```

---

## 故障排查

### 常见问题

#### 1. 端口占用
**错误**: "Address already in use"
**解决**: 停止占用端口的进程或修改端口配置

#### 2. 数据库锁定
**错误**: "Database is locked"
**解决**: 确保测试使用独立路径，检查进程是否正常退出

#### 3. Python依赖缺失
**错误**: ModuleNotFoundError
**解决**: 安装依赖
```bash
pip3 install requests grpcio grpcio-tools
```

#### 4. Protobuf文件不匹配
**错误**: gRPC调用失败
**解决**: 重新生成Python protobuf文件
```bash
cd tests_python
python3 -m grpc_tools.protoc -I../src/access/proto \
    --python_out=. --grpc_python_out=. \
    ../src/access/proto/rpc.proto
```

### 查看详细错误
```bash
# Rust
RUST_BACKTRACE=1 cargo test

# Python
python3 tests_python/test_e2e_grpc.py
python3 tests_python/test_e2e_rest.py
```

---

## CI/CD 集成

### GitHub Actions 示例

```yaml
name: Full Test Suite

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest

    steps:
    - uses: actions/checkout@v3

    - name: Install Rust
      uses: actions-rs/toolchain@v1
      with:
        toolchain: stable

    - name: Install Python
      uses: actions/setup-python@v4
      with:
        python-version: '3.9'

    - name: Install Python dependencies
      run: |
        pip3 install requests grpcio grpcio-tools protobuf

    - name: Run Rust tests
      run: cargo test

    - name: Run Python tests
      run: |
        cargo build --release
        ./run_all_tests.sh
```

---

## 相关文档

- [REST_API.md](REST_API.md) - REST API文档
- [gRPC_API.md](gRPC_API.md) - gRPC API文档
- [LSQL_USAGE.md](LSQL_USAGE.md) - lsql客户端使用文档
- [LAOFLCHDB_USAGE.md](LAOFLCHDB_USAGE.md) - 服务端工具使用文档
- [TEST_REPORT.md](TEST_REPORT.md) - 测试报告
- [README.md](README.md) - 项目README
