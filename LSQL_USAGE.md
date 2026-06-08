# lsql 使用说明

`lsql` 是 laoflchDB 的交互式 SQL 命令行客户端，类似 PostgreSQL 的 `psql`，通过 gRPC 连接到数据库服务。

---

## 目录

1. [概述](#概述)
2. [编译与安装](#编译与安装)
3. [命令行参数](#命令行参数)
4. [交互式命令](#交互式命令)
5. [查询结果显示](#查询结果显示)
6. [使用示例](#使用示例)
7. [常见问题](#常见问题)

---

## 概述

`lsql` 提供以下核心功能：
- 交互式 SQL 执行环境
- Schema 管理（查看、切换）
- 表管理（查看表列表、表结构）
- 命令历史记录
- 友好的错误提示

---

## 编译与安装

### 编译

```bash
# 编译 lsql 客户端
cargo build --bin lsql

# 编译优化版本
cargo build --release --bin lsql
```

### 运行位置

编译后的二进制文件位于：
- 调试版本: `target/debug/lsql`
- 发布版本: `target/release/lsql`

---

## 命令行参数

### 基本语法

```bash
lsql --host <host:port> [--schema <schema_name>] [--user <username>] [--password <password>] [--command <sql>]
```

### 参数说明

| 参数 | 简写 | 必填 | 说明 |
|------|------|------|------|
| `--host` | - | **是** | 数据库服务器地址，格式为 `host:port` |
| `--schema` | `-s` | 否 | 默认连接的 Schema 名称，默认为 `sys` |
| `--user` | `-u` | 否 | 用户名，用于登录认证 |
| `--password` | `-W` | 否 | 密码，用于登录认证 |
| `--command` | `-c` | 否 | 执行单次 SQL 命令后退出 |
| `--help` | `-h` | 否 | 显示帮助信息 |
| `--version` | `-V` | 否 | 显示版本信息 |

### 环境变量

| 环境变量 | 说明 |
|----------|------|
| `LAOFLCHDB_USER` | 用户名 |
| `LAOFLCHDB_PASSWORD` | 密码 |

### 使用示例

```bash
# 连接到本地数据库（必须指定 --host）
lsql --host 127.0.0.1:19777

# 使用用户名和密码登录
lsql --host 127.0.0.1:19777 --user admin --password admin123

# 使用简写参数
lsql --host 127.0.0.1:19777 -u admin -W admin123

# 使用环境变量登录
LAOFLCHDB_USER=admin LAOFLCHDB_PASSWORD=admin123 lsql --host 127.0.0.1:19777

# 连接到远程服务器并指定 Schema
lsql --host 192.168.1.100:19777 --schema analytics --user admin --password admin123

# 执行单次 SQL 查询
lsql --host 127.0.0.1:19777 --user admin --password admin123 --command "SELECT COUNT(*) FROM users"

# 显示帮助信息
lsql --help
```

---

## 交互式命令

进入交互式模式后，提示符显示当前 Schema 名称：`lsql@<schema_name>`

### 元命令列表

| 命令 | 别名 | 说明 |
|------|------|------|
| `\q` | `\quit` | 退出 lsql |
| `\help` | `\?`, `\h` | 显示帮助信息 |
| `\version` | - | 显示客户端和服务端版本信息 |
| `\dn` | `\schemas` | 列出所有可用的 Schema |
| `\dt` | - | 列出当前 Schema 中的所有表 |
| `\c <schema>` | `\connect <schema>` | 切换到指定的 Schema |
| `\d <table>` | - | 显示表结构（列名、类型等） |

### SQL 命令

直接输入 SQL 语句并回车执行：

```sql
lsql@sys> SELECT id, name FROM users WHERE age > 18;
lsql@sys> INSERT INTO products (name, price) VALUES ('Apple', 99.9);
lsql@sys> UPDATE users SET name = 'NewName' WHERE id = 1;
lsql@sys> DELETE FROM logs WHERE created_at < '2024-01-01';
```

---

## 查询结果显示

lsql 会以表格形式显示 SQL 查询结果，具有以下特性：

### 格式化特性

| 特性 | 说明 |
|------|------|
| **自动列宽调整** | 根据内容自动计算每列宽度 |
| **智能对齐** | 数字右对齐，字符串左对齐 |
| **浮点数优化** | 自动去除尾部零，最多显示6位小数 |
| **长字符串截断** | 超过50字符的字符串会自动截断并显示 `...` |
| **NULL 值显示** | 明确显示 `NULL` 标识 |
| **字节数组显示** | 以 `<bytes:长度>` 格式显示 |

### 示例输出

```
lsql@example> SELECT * FROM users;

+----+-------+---------------------+------------+
| id | name  | email               | created_at |
+----+-------+---------------------+------------+
|  1 | Alice | alice@example.com   | 2024-01-01 |
|  2 | Bob   | bob@example.com     | 2024-01-02 |
|  3 | Carol | carol@example.com   | 2024-01-03 |
+----+-------+---------------------+------------+
(3 行)

耗时: 2.345ms
```

### 数字对齐示例

数字类型自动右对齐，便于数值比较：

```
lsql@example> SELECT id, price, quantity FROM products;

+----+--------+----------+
| id | price  | quantity |
+----+--------+----------+
|  1 |  99.99 |      100 |
|  2 | 149.5  |       50 |
|  3 |   29.9 |      200 |
+----+--------+----------+
(3 行)
```

---

## 使用示例

### 示例 1：基本连接和查询

```bash
$ lsql --host 127.0.0.1:19777
欢迎使用 lsql - LaoflchDB SQL 客户端
正在连接到 127.0.0.1:19777...
连接成功！
默认 Schema: sys

输入 '\help' 查看帮助，'\q' 或 '\quit' 退出，'\dt' 查看所有表

lsql@sys> \dn
所有 Schema:
  - sys
  - analytics
  - test

lsql@sys> \dt
当前 Schema 'sys' 中的表:
  - users
  - products

lsql@sys> SELECT * FROM users LIMIT 3;
+----+-------+-----+
| id | name  | age |
+----+-------+-----+
| 1  | Alice | 30  |
| 2  | Bob   | 25  |
| 3  | Carol | 35  |
+----+-------+-----+
(3 行)
耗时: 12.345ms

lsql@sys> \q
再见！
```

### 示例 2：切换 Schema

```bash
$ lsql --host 127.0.0.1:19777
lsql@sys> \c analytics
已切换到 Schema 'analytics'
lsql@analytics> \dt
当前 Schema 'analytics' 中的表:
  - events
  - metrics

lsql@analytics> SELECT COUNT(*) FROM events;
+-------+
| count |
+-------+
| 1000  |
+-------+
(1 行)
```

### 示例 3：查看表结构

```bash
lsql@sys> \d user
表 "sys.user"
+--------+---------------+--------+
| 列ID   |     列名      | 类型   |
+--------+---------------+--------+
|      1 | id            | INT64  |
|      2 | username      | STRING |
|      3 | email         | STRING |
|      4 | password_hash | STRING |
|      5 | created_at    | STRING |
+--------+---------------+--------+
(5 列)
```

### 示例 4：执行单次命令

```bash
$ lsql --host 127.0.0.1:19777 --command "SELECT name, price FROM products WHERE price < 100"
欢迎使用 lsql - LaoflchDB SQL 客户端
正在连接到 127.0.0.1:19777...
连接成功！
默认 Schema: sys
+-------+-------+
| name  | price |
+-------+-------+
| Apple | 99.9  |
| Orange| 59.9  |
+-------+-------+
(2 行)
耗时: 8.123ms
```

---

## 常见问题

### Q1: 连接失败，提示 "connection refused"

**原因**：数据库服务未启动或地址/端口不正确

**解决方案**：
```bash
# 确保服务已启动
./target/release/laoflchDB-rust start

# 检查服务端口配置（默认端口为 19777）
cat laoflchdb.yaml | grep addr
```

### Q2: 提示 "Schema 'xxx' 不存在"

**原因**：指定的 Schema 不存在

**解决方案**：
```bash
# 先查看所有可用的 Schema
lsql --host 127.0.0.1:19777 --command "\dn"

# 或连接时使用存在的 Schema
lsql --host 127.0.0.1:19777 --schema sys
```

### Q3: SQL 执行错误，进程不会退出

**设计行为**：lsql 在 SQL 执行错误时只会打印错误信息，不会退出进程，允许继续执行其他命令

```sql
lsql@sys> SELECT * FROM non_existent_table;
错误: table 'non_existent_table' not found

lsql@sys> SELECT * FROM users;  -- 可以继续执行
```

### Q4: 默认 Schema 'sys' 不存在

**原因**：数据库未初始化或 sys Schema 被删除

**解决方案**：
```bash
# 初始化数据库（创建 sys Schema 和默认表）
./target/release/laoflchDB-rust init
```

---

## 版本信息

- **版本**: v0.1.3
- **协议**: gRPC
- **依赖**: `rustyline` (命令行交互), `tonic` (gRPC), `clap` (参数解析)

---

**文档版本**: v0.1.3  
**最后更新**: 2026-06-08  
**项目**: laoflchDB-rust