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
cargo test --test lsql_client_tests

# Python 自动化测试
python3 tests_python/test_e2e_grpc.py
python3 tests_python/test_e2e_rest.py
python3 tests_python/test_grpc_sql_query.py
python3 tests_python/test_grpc_sql_advanced.py
python3 tests_python/test_grpc_sql_join.py
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
- **内容**: 测试REST API接口的完整CRUD流程
- **测试场景**:
  1. 健康检查
  2. 创建表
  3. 列出表
  4. 获取表元数据
  5. 插入数据
  6. 读取数据
  7. 更新数据
  8. 删除数据
  9. 验证删除
  10. 错误处理

```bash
python3 tests_python/test_e2e_rest.py
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
| **REST** | `/health` | ✅ |
| **REST** | `/api/v1/tables` | ✅ |
| **REST** | `/api/v1/schemas/{schema}/tables` | ✅ |
| **REST** | `/api/v1/schemas/{schema}/tables/{table}` | ✅ |
| **REST** | `/api/v1/put` | ✅ |
| **REST** | `/api/v1/get` | ✅ |
| **REST** | `/api/v1/delete` | ✅ |

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
| Rust 单元测试 | 10 | ✅ |
| Rust SQL高级测试 | 6 | ✅ |
| Rust lsql客户端测试 | 4 | ✅ |
| Python gRPC测试 | 1 | ✅ |
| Python REST测试 | 10 | ✅ |
| Python SQL测试 | 3 | ✅ |
| **总计** | **34** | **✅** |

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
- [TEST_REPORT.md](TEST_REPORT.md) - 测试报告
- [README.md](README.md) - 项目README
