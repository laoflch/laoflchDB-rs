# laoflchdb 命令行工具使用说明

`laoflchdb` 是 LaoflchDB 数据库的服务端命令行工具，用于启动数据库服务和初始化数据库。

---

## 目录

1. [概述](#概述)
2. [编译与安装](#编译与安装)
3. [命令行参数](#命令行参数)
4. [子命令](#子命令)
5. [配置文件](#配置文件)
6. [使用示例](#使用示例)
7. [常见问题](#常见问题)

---

## 概述

`laoflchdb` 提供以下核心功能：
- 启动 LaoflchDB 数据库服务（支持 gRPC 和 REST API）
- 初始化数据库（创建默认 Schema 和表）
- 支持通过配置文件或命令行参数进行配置

---

## 编译与安装

### 编译

```bash
# 加载构建环境
source local.env

# 编译服务端
cargo build --release

# 编译后的二进制文件位置
# ./target/release/laoflchdb
```

---

## 命令行参数

### 基本语法

```bash
laoflchdb [OPTIONS] <COMMAND>
```

### 全局选项

| 参数 | 简写 | 必填 | 说明 |
|------|------|------|------|
| `--config` | `-c` | 否 | 配置文件路径 |
| `--help` | `-h` | 否 | 显示帮助信息 |
| `--version` | `-V` | 否 | 显示版本信息 |

---

## 子命令

### 1. `start` - 启动数据库服务

启动 LaoflchDB 数据库服务，同时提供 gRPC 和 REST API 接口。

```bash
laoflchdb start [OPTIONS]
```

**选项：**

| 参数 | 简写 | 必填 | 说明 |
|------|------|------|------|
| `--addr` | - | 否 | gRPC 服务监听地址，格式为 `host:port` |
| `--db-path` | - | 否 | 数据库数据存储路径 |

**默认值：**
- 监听地址：`127.0.0.1:19777`（gRPC），`127.0.0.1:8080`（REST）
- 数据库路径：`./laoflch_db_data`

### 2. `init` - 初始化数据库

初始化数据库，创建默认的 `sys` Schema 和 `user` 表。支持幂等执行。

```bash
laoflchdb init [OPTIONS]
```

**选项：**

| 参数 | 简写 | 必填 | 说明 |
|------|------|------|------|
| `--db-path` | - | 否 | 数据库数据存储路径 |
| `--example` | - | 否 | 是否同时初始化示例数据（会删除并重建 `example` Schema） |

**默认值：**
- 数据库路径：`./laoflch_db_data`
- 示例数据：不初始化

**幂等性说明：**

1. **`sys` Schema**：采用"存在则跳过"策略
   - 如果 `sys` Schema 已存在，不会删除或修改现有数据
   - 如果 `sys` Schema 不存在，会创建并创建 `user` 表
   - 适合在生产环境重复执行

2. **`example` Schema**（`--example` 选项）：采用"删除重建"策略
   - 如果 `example` Schema 已存在，会先删除再重新创建
   - 每次执行都会生成干净的示例数据
   - 适合开发测试环境

---

## 配置文件

### 配置文件格式

配置文件为 YAML 格式，示例如下：

```yaml
# laoflchdb.yaml
db_path: ./laoflch_db_data    # 数据库根目录
log_level: info              # 日志级别

access_protocols:
  - protocol: grpc
    enabled: true
    addr: 127.0.0.1:19777
    service_id: grpc_admin

  - protocol: rest
    enabled: true
    addr: 127.0.0.1:8080
    service_id: rest_admin

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
      - sql_query
```

### 配置项说明

| 配置项 | 类型 | 说明 |
|--------|------|------|
| `db_path` | string | 数据库数据存储根目录 |
| `log_level` | string | 日志级别（trace/debug/info/warn/error） |
| `access_protocols` | array | 访问协议配置列表 |
| `access_protocols[].protocol` | string | 协议类型（grpc/rest） |
| `access_protocols[].enabled` | bool | 是否启用 |
| `access_protocols[].addr` | string | 监听地址 |
| `access_protocols[].service_id` | string | 服务标识 |
| `permissions` | array | 权限配置列表 |
| `permissions[].service_id` | string | 服务标识 |
| `permissions[].default_policy` | string | 默认策略（allow/deny） |
| `permissions[].allowed_actions` | array | 允许的操作列表 |

---

## 使用示例

### 示例 1：初始化数据库

```bash
# 初始化数据库（使用默认路径）
./target/release/laoflchdb init

# 初始化数据库并指定路径
./target/release/laoflchdb init --db-path /data/laoflchdb

# 初始化数据库并创建示例数据
./target/release/laoflchdb init --example

# 使用配置文件初始化
./target/release/laoflchdb -c laoflchdb.yaml init
```

### 示例 2：启动数据库服务

```bash
# 使用默认配置启动服务
./target/release/laoflchdb start

# 指定监听地址和数据库路径
./target/release/laoflchdb start --addr 0.0.0.0:19777 --db-path /data/laoflchdb

# 使用配置文件启动服务
./target/release/laoflchdb -c laoflchdb.yaml start

# 后台启动（Linux）
nohup ./target/release/laoflchdb start > laoflchdb.log 2>&1 &
```

### 示例 3：查看帮助信息

```bash
# 查看全局帮助
./target/release/laoflchdb --help

# 查看 start 命令帮助
./target/release/laoflchdb start --help

# 查看 init 命令帮助
./target/release/laoflchdb init --help
```

---

## 服务启动后

服务启动后会监听以下端口：

| 协议 | 默认地址 | 说明 |
|------|----------|------|
| gRPC | `127.0.0.1:19777` | 高性能 RPC 接口 |
| REST | `127.0.0.1:8080` | HTTP REST API 接口 |

### 验证服务启动

```bash
# 检查 gRPC 服务
curl -s http://localhost:19777/health

# 检查 REST API
curl -s http://localhost:8080/health
```

### 用户认证

数据库初始化后会自动创建默认管理员用户：
- **用户名**: `admin`
- **密码**: `admin123`

**登录获取 Token**:

```bash
# 通过 REST API 登录
curl -X POST http://localhost:8080/api/v1/login \
  -H "Content-Type: application/json" \
  -d '{"username": "admin", "password": "admin123"}'

# 响应示例
# {
#   "success": true,
#   "message": "",
#   "data": {
#     "success": true,
#     "message": "Login successful",
#     "token": "550e8400-e29b-41d4-a716-446655440000",
#     "user_id": 1,
#     "username": "admin"
#   }
# }
```

**使用 Token 访问受保护的 API**:

```bash
# 使用获取到的 token
TOKEN="550e8400-e29b-41d4-a716-446655440000"

# 访问需要认证的接口
curl -H "Authorization: Bearer $TOKEN" \
  http://localhost:8080/api/v1/schemas/sys/tables
```

### 使用 lsql 客户端连接

```bash
# 使用 lsql 客户端连接（需要登录）
./target/release/lsql --host 127.0.0.1:19777 --user admin --password admin123

# 或使用环境变量
LAOFLCHDB_USER=admin LAOFLCHDB_PASSWORD=admin123 ./target/release/lsql --host 127.0.0.1:19777
```

---

## 常见问题

### Q1: 启动服务时提示 "address already in use"

**原因**：端口已被其他进程占用

**解决方案**：
```bash
# 查找占用端口的进程
lsof -i :19777

# 杀死进程
kill -9 <PID>

# 或使用其他端口启动
./target/release/laoflchdb start --addr 127.0.0.1:19778
```

### Q2: 初始化数据库失败，提示权限不足

**原因**：目标目录没有写权限

**解决方案**：
```bash
# 创建目录并设置权限
mkdir -p /data/laoflchdb
chown -R user:group /data/laoflchdb
chmod -R 755 /data/laoflchdb

# 然后重新初始化
./target/release/laoflchdb init --db-path /data/laoflchdb
```

### Q3: 服务启动后无法连接

**原因**：可能是防火墙阻止或监听地址配置问题

**解决方案**：
```bash
# 检查监听状态
netstat -tlnp | grep 19777

# 确保监听的是 0.0.0.0 而不是 127.0.0.1（允许外部访问）
./target/release/laoflchdb start --addr 0.0.0.0:19777

# 检查防火墙规则
firewall-cmd --list-ports
firewall-cmd --add-port=19777/tcp --permanent
firewall-cmd --reload
```

### Q4: 数据库文件损坏

**原因**：异常关机或磁盘错误

**解决方案**：
```bash
# 备份损坏的数据
cp -r /data/laoflchdb /data/laoflchdb_backup

# 删除损坏的数据目录
rm -rf /data/laoflchdb

# 重新初始化
./target/release/laoflchdb init --db-path /data/laoflchdb
```

---

## 版本信息

- **版本**: v0.1.0
- **协议**: gRPC + REST
- **存储**: RocksDB

---

**文档版本**: v0.1.0  
**最后更新**: 2026-06-08  
**项目**: laoflchDB-rust