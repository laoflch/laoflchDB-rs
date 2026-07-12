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
python3 tests_python/test_vector_service_grpc.py  # 向量化服务测试
python3 tests_python/test_embedding_service_grpc.py  # 嵌入向量索引服务测试
python3 tests_python/test_object_store_service_grpc.py  # 对象存储服务 gRPC 测试
python3 tests_python/test_object_store_service_rest.py  # 对象存储服务 REST 测试（S3 兼容性）
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

#### 向量化服务 gRPC 测试
- **文件**: `tests_python/test_vector_service_grpc.py`
- **内容**: 测试向量化服务的完整功能
- **测试场景**:
  1. 创建文本向量（CreateEmbedding with texts）
  2. 创建图片向量（CreateEmbedding with images）
  3. 向量 L2 归一化验证
  4. 向量化结果确定性验证
  5. 计算向量相似度
  6. 加载/卸载模型
  7. 列出已加载模型
  8. 列出可加载模型
  9. 获取模型信息
  10. 模型类型检测（文本/视觉）

```bash
python3 tests_python/test_vector_service_grpc.py
```

#### 嵌入向量索引服务 gRPC 测试
- **文件**: `tests_python/test_embedding_service_grpc.py`
- **内容**: 测试嵌入向量索引服务的完整功能
- **测试场景**:
  1. 插入向量到索引
  2. 搜索 Top-K 最近邻
  3. 搜索结果准确性验证
  4. 从索引中删除向量
  5. 获取索引统计信息
  6. 索引快照保存和加载
  7. 批量插入向量
  8. 范围搜索

```bash
python3 tests_python/test_embedding_service_grpc.py
```

#### 对象存储服务 gRPC 测试
- **文件**: `tests_python/test_object_store_service_grpc.py`
- **内容**: 测试对象存储服务 gRPC 接口的完整功能（S3 兼容 API）
- **测试场景**:
  1. 创建 Bucket
  2. 重复创建 Bucket（幂等）
  3. 列出 Buckets
  4. 存储对象（PutObject）
  5. 存储大对象（1MB，验证 BlobDB 大对象存储）
  6. 存储空对象
  7. 存储特殊字符路径对象
  8. 带自定义元数据存储对象
  9. 覆盖已有对象
  10. 向不存在 Bucket 存储对象
  11. 获取对象（GetObject）
  12. 获取大对象
  13. 获取空对象
  14. 获取不存在对象
  15. 获取对象元数据（HeadObject）
  16. 获取不存在对象元数据
  17. 列出对象（ListObjects）
  18. 带前缀列出对象
  19. 带分隔符列出对象（模拟目录结构）
  20. 空 Bucket 列出对象
  21. 复制对象（CopyObject）
  22. 跨 Bucket 复制对象
  23. 删除对象（DeleteObject）
  24. 删除不存在对象（幂等）
  25. 批量删除对象（DeleteObjects）
  26. 批量删除部分不存在对象
  27. 删除后列出对象验证
  28. 删除 Bucket
  29. 创建无效 Bucket 名称

```bash
python3 tests_python/test_object_store_service_grpc.py
```

#### 对象存储服务 REST 测试（S3 兼容性）
- **文件**: `tests_python/test_object_store_service_rest.py`
- **内容**: 测试对象存储服务 S3 兼容的 REST API 接口（`/api/v1/object-store` 前缀下的 HTTP 端点）
- **默认端口**: `8080`（通过环境变量 `LAOFLCHDB_REST_PORT` 可覆盖）
- **测试场景**:
  1. 用户登录获取 Token
  2. ListBuckets - 列出所有 Bucket（GET /）
  3. CreateBucket - 创建 Bucket（PUT /{bucket}）
  4. CreateBucket 幂等性验证
  5. PutObject - 上传普通对象（PUT /{bucket}/{key}）
  6. PutObject 大对象上传（1MB，验证 BlobDB）
  7. PutObject 空对象上传
  8. PutObject 特殊字符路径（URL 编码）
  9. PutObject 覆盖已有对象
  10. GetObject - 下载对象（GET /{bucket}/{key}）
  11. GetObject 大对象下载
  12. GetObject 空对象下载
  13. GetObject 不存在对象（应返回 404）
  14. HeadObject - 获取对象元数据（HEAD /{bucket}/{key}）
  15. HeadObject 不存在对象（应返回 404）
  16. ListObjects - 列出对象（GET /{bucket}）
  17. ListObjects 带前缀过滤
  18. ListObjects 带分隔符（模拟目录结构，验证 common_prefixes）
  19. ListObjects 空 Bucket
  20. ListObjects 带 max_keys 分页
  21. DeleteObject - 删除对象（DELETE /{bucket}/{key}）
  22. DeleteObject 幂等性验证
  23. DeleteBucket - 删除 Bucket（DELETE /{bucket}）
  24. DeleteBucket 不存在 Bucket（应返回 500）
  25. PutObject 向不存在 Bucket 上传（应自动创建 Bucket）
  26. Content-Type 保留验证
  27. ETag 一致性验证
  28. 删除后列出对象验证
  29. REST 健康检查

```bash
python3 tests_python/test_object_store_service_rest.py
```

**说明**: REST 测试覆盖了 S3 兼容的所有 HTTP 端点，包括 ListBuckets、CreateBucket、DeleteBucket、ListObjects、PutObject、GetObject、HeadObject、DeleteObject。测试验证了 Bucket CRUD、Object CRUD、大对象上传/下载、特殊字符路径、ETag 一致性、Content-Type 保留等关键功能。

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
| **gRPC** | CreateEmbedding | ✅ |
| **gRPC** | ComputeSimilarity | ✅ |
| **gRPC** | LoadModel | ✅ |
| **gRPC** | UnloadModel | ✅ |
| **gRPC** | ListModels | ✅ |
| **gRPC** | ListLoadableModels | ✅ |
| **gRPC** | GetModelInfo | ✅ |
| **gRPC** | InsertEmbedding | ✅ |
| **gRPC** | SearchEmbedding | ✅ |
| **gRPC** | DeleteEmbedding | ✅ |
| **gRPC** | GetIndexInfo | ✅ |
| **gRPC** | SaveSnapshot | ✅ |
| **gRPC** | LoadSnapshot | ✅ |
| **gRPC** | PutObject | ✅ |
| **gRPC** | GetObject | ✅ |
| **gRPC** | DeleteObject | ✅ |
| **gRPC** | ListObjects | ✅ |
| **gRPC** | HeadObject | ✅ |
| **gRPC** | CopyObject | ✅ |
| **gRPC** | DeleteObjects | ✅ |
| **gRPC** | CreateBucket | ✅ |
| **gRPC** | DeleteBucket | ✅ |
| **gRPC** | ListBuckets | ✅ |
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
| **REST** | `/api/v1/object-store` (ListBuckets, GET) | ✅ |
| **REST** | `/api/v1/object-store/{bucket}` (CreateBucket, PUT) | ✅ |
| **REST** | `/api/v1/object-store/{bucket}` (ListObjects, GET) | ✅ |
| **REST** | `/api/v1/object-store/{bucket}` (DeleteBucket, DELETE) | ✅ |
| **REST** | `/api/v1/object-store/{bucket}/{key}` (PutObject, PUT) | ✅ |
| **REST** | `/api/v1/object-store/{bucket}/{key}` (GetObject, GET) | ✅ |
| **REST** | `/api/v1/object-store/{bucket}/{key}` (HeadObject, HEAD) | ✅ |
| **REST** | `/api/v1/object-store/{bucket}/{key}` (DeleteObject, DELETE) | ✅ |

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
| 文本向量化 | ✅ |
| 图片向量化 | ✅ |
| 向量 L2 归一化 | ✅ |
| 模型加载/卸载 | ✅ |
| 模型自动检测 | ✅ |
| 向量相似度计算 | ✅ |
| ANN 向量搜索 | ✅ |
| 向量持久化存储 | ✅ |
| 索引快照管理 | ✅ |
| 批量向量插入 | ✅ |
| 对象存储 Bucket 管理 | ✅ |
| 对象存储/获取/删除 | ✅ |
| 对象存储大对象（BlobDB） | ✅ |
| 对象存储元数据管理 | ✅ |
| 对象存储批量删除 | ✅ |
| 对象存储跨 Bucket 复制 | ✅ |
| 对象存储目录结构模拟 | ✅ |

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
   - 向量化服务测试
   - 嵌入向量索引服务测试
   - 对象存储服务 gRPC 测试
   - 对象存储服务 REST 测试（S3 兼容性）

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
| Python 向量化服务测试 | 1 | ✅ |
| Python 嵌入向量索引服务测试 | 1 | ✅ |
| Python 对象存储服务 gRPC 测试 | 1 | ✅ |
| Python 对象存储服务 REST 测试 | 1 | ✅ |
| **总计** | **61** | **✅** |

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
