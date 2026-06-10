#!/usr/bin/env python3
"""
gRPC SQL 跨 Schema JOIN 测试
- 创建两个新 schema
- 单 schema 全量 SQL 测试
- 跨 schema JOIN 测试
"""
import subprocess
import time
import sys
import os
import signal
import grpc

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import rpc_pb2
import rpc_pb2_grpc
import field_pb2

def encode_field(value, field_type):
    """将值编码为 protobuf Field 对象"""
    field = field_pb2.Field()
    if field_type == 0:  # STRING
        field.string_value.value = value
    elif field_type == 1:  # INT64
        field.integer_value.value = int(value)
    elif field_type == 3:  # FLOAT
        field.float_value.value = float(value)
    elif field_type == 2:  # BYTES
        field.bytes_value.value = value if isinstance(value, bytes) else value.encode()
    return field.SerializeToString()

TEST_DB = "./laoflch_db_cross_schema_test"
TEST_ADDR = "127.0.0.1:19777"
SERVER_BIN = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "release", "laoflchdb")


def needs_rebuild():
    """检查是否需要重新编译 Rust 代码"""
    server_bin = SERVER_BIN
    cargo_lock = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "Cargo.lock")

    # 检查 binary 是否存在
    if not os.path.exists(server_bin):
        return True

    # 检查 Cargo.lock 是否比 binary 更新
    if os.path.exists(cargo_lock):
        bin_mtime = os.path.getmtime(server_bin)
        lock_mtime = os.path.getmtime(cargo_lock)
        if lock_mtime > bin_mtime:
            return True

    return False


def run_cross_schema_join_test():
    os.chdir(os.path.dirname(os.path.abspath(__file__)))

    print("=" * 70)
    print("Python 自动回归测试: 跨 Schema JOIN 查询")
    print("=" * 70)

    print("\n[1/8] 检查是否需要编译 Rust...")
    if needs_rebuild():
        print("    需要重新编译 Rust...")
        result = subprocess.run(["cargo", "build", "--release", "-j", "12"], cwd="..", capture_output=True)
        if result.returncode != 0:
            print("编译失败:", result.stderr.decode())
            return 1
        print("    ✓ 编译完成")
    else:
        print("    ✓ Rust 代码未修改，跳过编译")

    print("\n[2/8] 停止可能存在的旧服务...")
    subprocess.run(["pkill", "-9", "-f", "laoflchdb"], capture_output=True)
    time.sleep(2)
    print("    ✓ 已停止旧服务")

    print("\n[3/8] 启动 laoflchDB gRPC 服务（使用配置文件和已有数据库）...")
    config_path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "laoflchdb.yaml")
    cmd = [
        SERVER_BIN,
        "-c", config_path,
        "start"
    ]
    server_proc = subprocess.Popen(
        cmd,
        cwd=os.path.dirname(os.path.abspath(__file__)),
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        preexec_fn=os.setsid
    )
    time.sleep(6)
    print(f"    ✓ 服务已启动 PID={server_proc.pid}")

    try:
        print("\n[4/8] 等待服务就绪...")
        max_retries = 10
        for i in range(max_retries):
            try:
                channel = grpc.insecure_channel(TEST_ADDR)
                stub = rpc_pb2_grpc.LaoflchDbStub(channel)
                stub.SqlQuery(rpc_pb2.SqlQueryRequest(
                    schema="sys",
                    sql="SELECT 1"
                ), timeout=2)
                print(f"    ✓ 服务已就绪 (尝试 {i+1}/{max_retries})")
                break
            except grpc.RpcError as e:
                print(f"    服务尚未就绪 (尝试 {i+1}/{max_retries}): {e.code()}")
                time.sleep(1)
                continue
        else:
            print("    ✗ 服务启动失败")
            return 1

        # ====== 创建两个新 Schema ======
        print("\n[5/8] 创建测试 Schema...")

        schema1 = "sales"
        schema2 = "inventory"

        print(f"    创建 schema '{schema1}'...")
        resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql=f"CREATE SCHEMA IF NOT EXISTS {schema1}"
        ))
        print(f"        结果: {resp.success} - {resp.message}")

        print(f"    创建 schema '{schema2}'...")
        resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql=f"CREATE SCHEMA IF NOT EXISTS {schema2}"
        ))
        print(f"        结果: {resp.success} - {resp.message}")

        time.sleep(2)

        # ====== 创建 sales schema 的表 ======
        print(f"\n    在 '{schema1}' 中创建表...")

        # 先删除可能存在的表
        for table in ["orders", "customers"]:
            try:
                stub.DropTable(rpc_pb2.DropTableRequest(
                    schema=schema1,
                    table_name=table
                ))
                print(f"        已删除旧表 '{table}'")
            except:
                pass

        # orders 表
        stub.CreateTable(rpc_pb2.CreateTableRequest(
            schema=schema1,
            table_name="orders",
            columns=[
                rpc_pb2.ColumnDef(name="order_id", column_type=1),     # INT64
                rpc_pb2.ColumnDef(name="customer_id", column_type=1),  # INT64
                rpc_pb2.ColumnDef(name="product_id", column_type=1),   # INT64
                rpc_pb2.ColumnDef(name="quantity", column_type=1),      # INT64
                rpc_pb2.ColumnDef(name="total_price", column_type=3),   # FLOAT
                rpc_pb2.ColumnDef(name="order_date", column_type=0),    # STRING
            ]
        ))
        print("        ✓ 创建 orders 表成功")

        # customers 表
        stub.CreateTable(rpc_pb2.CreateTableRequest(
            schema=schema1,
            table_name="customers",
            columns=[
                rpc_pb2.ColumnDef(name="customer_id", column_type=1),  # INT64
                rpc_pb2.ColumnDef(name="name", column_type=0),        # STRING
                rpc_pb2.ColumnDef(name="city", column_type=0),        # STRING
            ]
        ))
        print("        ✓ 创建 customers 表成功")

        # ====== 创建 inventory schema 的表 ======
        print(f"\n    在 '{schema2}' 中创建表...")

        # 先删除可能存在的表
        for table in ["products", "warehouses"]:
            try:
                stub.DropTable(rpc_pb2.DropTableRequest(
                    schema=schema2,
                    table_name=table
                ))
                print(f"        已删除旧表 '{table}'")
            except:
                pass

        # products 表
        stub.CreateTable(rpc_pb2.CreateTableRequest(
            schema=schema2,
            table_name="products",
            columns=[
                rpc_pb2.ColumnDef(name="product_id", column_type=1),   # INT64
                rpc_pb2.ColumnDef(name="name", column_type=0),         # STRING
                rpc_pb2.ColumnDef(name="price", column_type=3),        # FLOAT
                rpc_pb2.ColumnDef(name="stock", column_type=1),        # INT64
                rpc_pb2.ColumnDef(name="category", column_type=0),     # STRING
            ]
        ))
        print("        ✓ 创建 products 表成功")

        # warehouses 表
        stub.CreateTable(rpc_pb2.CreateTableRequest(
            schema=schema2,
            table_name="warehouses",
            columns=[
                rpc_pb2.ColumnDef(name="warehouse_id", column_type=1), # INT64
                rpc_pb2.ColumnDef(name="location", column_type=0),     # STRING
                rpc_pb2.ColumnDef(name="capacity", column_type=1),     # INT64
            ]
        ))
        print("        ✓ 创建 warehouses 表成功")

        time.sleep(2)

        # ====== 插入测试数据到 sales ======
        print(f"\n[6/8] 插入测试数据...")

        customers_data = [
            (1, "Alice", "New York"),
            (2, "Bob", "London"),
            (3, "Charlie", "Paris"),
            (4, "David", "Tokyo"),
            (5, "Eve", "Berlin"),
        ]

        for cid, name, city in customers_data:
            stub.AddRow(rpc_pb2.AddRowRequest(
                schema=schema1,
                table_name="customers",
                row=rpc_pb2.Row(
                    row_type=0, version=1,
                    data=[
                        encode_field(cid, 1),
                        encode_field(name, 0),
                        encode_field(city, 0),
                    ]
                )
            ))
        print(f"        ✓ 插入 {len(customers_data)} 条 customers 数据")

        orders_data = [
            (101, 1, 1, 2, 199.99, "2026-01-15"),
            (102, 1, 3, 1, 599.99, "2026-01-20"),
            (103, 2, 2, 3, 149.97, "2026-02-10"),
            (104, 3, 1, 1, 99.99, "2026-02-15"),
            (105, 4, 4, 5, 249.95, "2026-03-01"),
            (106, 5, 2, 2, 99.98, "2026-03-05"),
            (107, 2, 1, 4, 399.98, "2026-03-10"),
            (108, 3, 3, 2, 1199.98, "2026-03-15"),
        ]

        for oid, cid, pid, qty, price, date in orders_data:
            stub.AddRow(rpc_pb2.AddRowRequest(
                schema=schema1,
                table_name="orders",
                row=rpc_pb2.Row(
                    row_type=0, version=1,
                    data=[
                        encode_field(oid, 1),
                        encode_field(cid, 1),
                        encode_field(pid, 1),
                        encode_field(qty, 1),
                        encode_field(price, 3),
                        encode_field(date, 0),
                    ]
                )
            ))
        print(f"        ✓ 插入 {len(orders_data)} 条 orders 数据")

        # ====== 插入测试数据到 inventory ======
        print(f"\n    插入数据到 '{schema2}'...")

        products_data = [
            (1, "Laptop", 999.99, 50, "Electronics"),
            (2, "Keyboard", 49.99, 200, "Electronics"),
            (3, "Monitor", 599.99, 30, "Electronics"),
            (4, "Desk Chair", 49.99, 100, "Furniture"),
            (5, "Standing Desk", 299.99, 25, "Furniture"),
        ]

        for pid, name, price, stock, cat in products_data:
            stub.AddRow(rpc_pb2.AddRowRequest(
                schema=schema2,
                table_name="products",
                row=rpc_pb2.Row(
                    row_type=0, version=1,
                    data=[
                        encode_field(pid, 1),
                        encode_field(name, 0),
                        encode_field(price, 3),
                        encode_field(stock, 1),
                        encode_field(cat, 0),
                    ]
                )
            ))
        print(f"        ✓ 插入 {len(products_data)} 条 products 数据")

        warehouses_data = [
            (1, "New York Warehouse", 10000),
            (2, "Los Angeles Warehouse", 15000),
            (3, "Chicago Warehouse", 8000),
        ]

        for wid, loc, cap in warehouses_data:
            stub.AddRow(rpc_pb2.AddRowRequest(
                schema=schema2,
                table_name="warehouses",
                row=rpc_pb2.Row(
                    row_type=0, version=1,
                    data=[
                        encode_field(wid, 1),
                        encode_field(loc, 0),
                        encode_field(cap, 1),
                    ]
                )
            ))
        print(f"        ✓ 插入 {len(warehouses_data)} 条 warehouses 数据")

        time.sleep(2)

        # ====== Schema 1 (sales) 单表全量 SQL 测试 ======
        print("\n[7/8] 单 schema SQL 测试...")
        print("\n" + "=" * 70)
        print("【Schema 1: sales 单表 SQL 测试】")
        print("=" * 70)

        print("\n    1.1 SELECT * FROM sales.customers:")
        resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(
            schema=schema1,
            sql="SELECT * FROM sales.customers"
        ))
        assert resp.success, f"查询失败: {resp.message}"
        print(f"        ✓ 返回 {len(resp.rows)} 行")

        print("\n    1.2 SELECT * FROM customers (不带 schema 前缀):")
        resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(
            schema=schema1,
            sql="SELECT * FROM customers"
        ))
        assert resp.success, f"查询失败: {resp.message}"
        print(f"        ✓ 返回 {len(resp.rows)} 行")

        print("\n    1.3 SELECT * FROM sales.orders:")
        resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(
            schema=schema1,
            sql="SELECT * FROM sales.orders"
        ))
        assert resp.success, f"查询失败: {resp.message}"
        print(f"        ✓ 返回 {len(resp.rows)} 行")

        print("\n    1.4 SELECT with WHERE condition:")
        resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(
            schema=schema1,
            sql="SELECT * FROM sales.orders WHERE customer_id = 1"
        ))
        assert resp.success, f"查询失败: {resp.message}"
        print(f"        ✓ 返回 {len(resp.rows)} 行")

        print("\n    1.5 SELECT with ORDER BY:")
        resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(
            schema=schema1,
            sql="SELECT * FROM sales.orders ORDER BY total_price DESC"
        ))
        assert resp.success, f"查询失败: {resp.message}"
        print(f"        ✓ 返回 {len(resp.rows)} 行")

        print("\n    1.6 SELECT with aggregate (COUNT):")
        resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(
            schema=schema1,
            sql="SELECT COUNT(*) FROM sales.orders"
        ))
        assert resp.success, f"查询失败: {resp.message}"
        print(f"        ✓ 返回 {len(resp.rows)} 行")

        print("\n    1.7 SELECT with GROUP BY:")
        resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(
            schema=schema1,
            sql="SELECT customer_id, COUNT(*) as order_count FROM sales.orders GROUP BY customer_id"
        ))
        assert resp.success, f"查询失败: {resp.message}"
        print(f"        ✓ 返回 {len(resp.rows)} 行")

        print("\n    1.8 SELECT with HAVING:")
        resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(
            schema=schema1,
            sql="SELECT customer_id, COUNT(*) as cnt FROM sales.orders GROUP BY customer_id HAVING COUNT(*) > 1"
        ))
        assert resp.success, f"查询失败: {resp.message}"
        print(f"        ✓ 返回 {len(resp.rows)} 行")

        # ====== Schema 2 (inventory) 单表全量 SQL 测试 ======
        print("\n" + "=" * 70)
        print("【Schema 2: inventory 单表 SQL 测试】")
        print("=" * 70)

        print("\n    2.1 SELECT * FROM inventory.products:")
        resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(
            schema=schema2,
            sql="SELECT * FROM inventory.products"
        ))
        assert resp.success, f"查询失败: {resp.message}"
        print(f"        ✓ 返回 {len(resp.rows)} 行")

        print("\n    2.2 SELECT * FROM products (不带 schema 前缀):")
        resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(
            schema=schema2,
            sql="SELECT * FROM products"
        ))
        assert resp.success, f"查询失败: {resp.message}"
        print(f"        ✓ 返回 {len(resp.rows)} 行")

        print("\n    2.3 SELECT * FROM inventory.warehouses:")
        resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(
            schema=schema2,
            sql="SELECT * FROM inventory.warehouses"
        ))
        assert resp.success, f"查询失败: {resp.message}"
        print(f"        ✓ 返回 {len(resp.rows)} 行")

        print("\n    2.4 SELECT with WHERE and ORDER BY:")
        resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(
            schema=schema2,
            sql="SELECT * FROM inventory.products WHERE stock > 50 ORDER BY price"
        ))
        assert resp.success, f"查询失败: {resp.message}"
        print(f"        ✓ 返回 {len(resp.rows)} 行")

        print("\n    2.5 SELECT with SUM aggregate:")
        resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(
            schema=schema2,
            sql="SELECT SUM(stock * price) as total_value FROM inventory.products"
        ))
        assert resp.success, f"查询失败: {resp.message}"
        print(f"        ✓ 返回 {len(resp.rows)} 行")

        # ====== 同 schema JOIN 测试 ======
        print("\n" + "=" * 70)
        print("【同 Schema JOIN 测试 (sales)】")
        print("=" * 70)

        print("\n    3.1 INNER JOIN orders + customers:")
        resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(
            schema=schema1,
            sql="SELECT o.order_id, c.name, o.total_price FROM sales.orders o INNER JOIN sales.customers c ON o.customer_id = c.customer_id"
        ))
        assert resp.success, f"查询失败: {resp.message}"
        print(f"        ✓ 返回 {len(resp.rows)} 行")

        print("\n    3.2 LEFT JOIN orders + customers:")
        resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(
            schema=schema1,
            sql="SELECT c.name, o.order_id FROM sales.customers c LEFT JOIN sales.orders o ON c.customer_id = o.customer_id"
        ))
        assert resp.success, f"查询失败: {resp.message}"
        print(f"        ✓ 返回 {len(resp.rows)} 行")

        # ====== 跨 Schema JOIN 测试 ======
        print("\n[8/8] 跨 schema JOIN 测试...")
        print("\n" + "=" * 70)
        print("【跨 Schema JOIN 测试 (sales ↔ inventory)】")
        print("=" * 70)

        print("\n    4.1 跨 Schema INNER JOIN (sales.orders + inventory.products):")
        resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(
            schema=schema1,
            sql="SELECT o.order_id, o.quantity, p.name, p.price, o.total_price "
                "FROM sales.orders o "
                "INNER JOIN inventory.products p ON o.product_id = p.product_id"
        ))
        assert resp.success, f"跨 schema JOIN 失败: {resp.message}"
        print(f"        ✓ 返回 {len(resp.rows)} 行")
        if len(resp.rows) > 0:
            print(f"        示例: {resp.rows[0]}")

        print("\n    4.2 跨 Schema LEFT JOIN:")
        resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(
            schema=schema1,
            sql="SELECT o.order_id, p.name as product_name, p.price "
                "FROM sales.orders o "
                "LEFT JOIN inventory.products p ON o.product_id = p.product_id"
        ))
        assert resp.success, f"跨 schema LEFT JOIN 失败: {resp.message}"
        print(f"        ✓ 返回 {len(resp.rows)} 行")

        print("\n    4.3 三表跨 Schema JOIN (orders + customers + products):")
        resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(
            schema=schema1,
            sql="SELECT c.name as customer, p.name as product, o.quantity, o.total_price "
                "FROM sales.orders o "
                "INNER JOIN sales.customers c ON o.customer_id = c.customer_id "
                "INNER JOIN inventory.products p ON o.product_id = p.product_id"
        ))
        assert resp.success, f"三表 JOIN 失败: {resp.message}"
        print(f"        ✓ 返回 {len(resp.rows)} 行")
        if len(resp.rows) > 0:
            print(f"        示例: {resp.rows[0]}")

        print("\n    4.4 跨 Schema JOIN with WHERE:")
        resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(
            schema=schema1,
            sql="SELECT o.order_id, p.name, p.price "
                "FROM sales.orders o "
                "INNER JOIN inventory.products p ON o.product_id = p.product_id "
                "WHERE p.category = 'Electronics'"
        ))
        assert resp.success, f"跨 schema WHERE JOIN 失败: {resp.message}"
        print(f"        ✓ 返回 {len(resp.rows)} 行")

        print("\n    4.5 跨 Schema JOIN with aggregate:")
        resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(
            schema=schema1,
            sql="SELECT p.category, COUNT(o.order_id) as order_count, SUM(o.total_price) as total "
                "FROM sales.orders o "
                "INNER JOIN inventory.products p ON o.product_id = p.product_id "
                "GROUP BY p.category"
        ))
        assert resp.success, f"跨 schema 聚合 JOIN 失败: {resp.message}"
        print(f"        ✓ 返回 {len(resp.rows)} 行")

        print("\n    4.6 跨 Schema JOIN 同一表的多个实例:")
        resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(
            schema=schema1,
            sql="SELECT o1.order_id as order1, o2.order_id as order2, p.name "
                "FROM sales.orders o1 "
                "INNER JOIN sales.orders o2 ON o1.customer_id = o2.customer_id AND o1.order_id < o2.order_id "
                "INNER JOIN inventory.products p ON o1.product_id = p.product_id"
        ))
        assert resp.success, f"自关联 JOIN 失败: {resp.message}"
        print(f"        ✓ 返回 {len(resp.rows)} 行")

        print("\n" + "=" * 70)
        print("SUCCESS! 跨 Schema JOIN 测试全部通过")
        print("=" * 70)
        print(f"测试数据保留在: {TEST_DB}")

        return 0

    except Exception as e:
        print(f"\n    ✗ 测试失败: {type(e).__name__}: {e}")
        import traceback
        traceback.print_exc()
        return 1
    finally:
        print("\n--- 终止服务进程 ---")
        try:
            import signal
            os.kill(server_proc.pid, signal.SIGTERM)
            server_proc.wait(timeout=3)
        except:
            try:
                server_proc.kill()
            except:
                pass
        print(f"测试数据保留在: {TEST_DB}")


if __name__ == "__main__":
    sys.exit(run_cross_schema_join_test())
