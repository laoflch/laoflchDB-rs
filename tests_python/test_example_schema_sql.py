#!/usr/bin/env python3
"""
gRPC SQL 查询测试 - 验证非 sys schema (example) 的单表和 JOIN 查询
使用已存在的 example schema 数据进行测试
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

TEST_ADDR = "127.0.0.1:19777"
SERVER_BIN = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "release", "laoflchdb")
# 使用已存在 example 数据的数据库路径
DB_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "release", "laoflch_db_data")


def run_example_schema_sql_test():
    os.chdir(os.path.dirname(os.path.abspath(__file__)))

    print("=" * 70)
    print("Python 自动回归测试: example schema SQL 查询")
    print("=" * 70)

    # 检查数据库路径是否存在
    if not os.path.exists(DB_PATH):
        print(f"\n错误: 数据库路径不存在: {DB_PATH}")
        print("请先运行 init 命令初始化数据库")
        return 1

    # 检查 example schema 是否存在
    example_path = os.path.join(DB_PATH, "example")
    if not os.path.exists(example_path):
        print(f"\n错误: example schema 不存在: {example_path}")
        print("请先运行 init 命令初始化数据库")
        return 1

    print(f"\n[1/5] 使用已有数据库: {DB_PATH}")

    print("\n[2/5] 停止可能存在的旧服务...")
    subprocess.run(["pkill", "-f", "laoflchdb"], capture_output=True)
    time.sleep(2)
    print("    ✓ 已停止旧服务")

    print("\n[3/5] 启动 laoflchDB gRPC 服务后台进程...")
    cmd = [
        SERVER_BIN,
        "-c", os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "laoflchdb.yaml"),
        "start"
    ]
    server_proc = subprocess.Popen(
        cmd,
        cwd=os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."),
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True
    )
    time.sleep(4)
    print(f"    ✓ 服务已启动 PID={server_proc.pid} 监听 {TEST_ADDR}")

    try:
        print("\n[4/5] 等待服务就绪...")
        max_retries = 10
        for i in range(max_retries):
            try:
                channel = grpc.insecure_channel(TEST_ADDR)
                stub = rpc_pb2_grpc.LaoflchDbStub(channel)
                # 尝试执行一个简单查询来验证服务已就绪
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

        print("\n[5/5] 测试 example schema SQL 查询...")

        # 先检查 example schema 中的表
        print("\n    检查 example schema 中的表...")
        list_resp = stub.ListTables(rpc_pb2.ListTablesRequest(schema="example"))
        print(f"    example schema 表列表: {list_resp.tables}")

        # 等待表注册到 SQL 引擎
        time.sleep(2)

        # ========== 单表 SQL 查询测试 ==========
        print("\n" + "-" * 50)
        print("【单表 SQL 查询测试】")
        print("-" * 50)

        # 测试 1: 查询 orders 表（带 schema 前缀）
        print("\n    1.1 测试 SELECT * FROM example.orders (带 schema 前缀):")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="example",
            sql="SELECT * FROM example.orders"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, f"查询失败: {sql_resp.message}"
        print(f"        ✓ 查询成功，返回 {len(sql_resp.rows)} 行")
        assert len(sql_resp.rows) > 0, "orders 表应该有数据"
        if len(sql_resp.rows) > 0:
            print(f"        示例数据: {sql_resp.rows[0]}")

        # 测试 2: 查询 orders 表（不带 schema 前缀）
        print("\n    1.2 测试 SELECT * FROM orders (不带 schema 前缀):")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="example",
            sql="SELECT * FROM orders"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, f"查询失败: {sql_resp.message}"
        print(f"        ✓ 查询成功，返回 {len(sql_resp.rows)} 行")

        # 测试 3: 带 WHERE 条件的查询
        print("\n    1.3 测试 SELECT * FROM orders WHERE status = 'COMPLETED':")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="example",
            sql="SELECT * FROM orders WHERE status = 'COMPLETED'"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, f"查询失败: {sql_resp.message}"
        print(f"        ✓ 查询成功，返回 {len(sql_resp.rows)} 行")

        # 测试 4: 带聚合函数的查询
        print("\n    1.4 测试 SELECT COUNT(*) FROM orders:")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="example",
            sql="SELECT COUNT(*) FROM orders"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, f"查询失败: {sql_resp.message}"
        print(f"        ✓ 查询成功，返回 {len(sql_resp.rows)} 行")
        if len(sql_resp.rows) > 0:
            print(f"        订单总数: {sql_resp.rows[0]}")

        # 测试 5: 查询 customers 表
        print("\n    1.5 测试 SELECT * FROM customers:")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="example",
            sql="SELECT * FROM customers"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, f"查询失败: {sql_resp.message}"
        print(f"        ✓ 查询成功，返回 {len(sql_resp.rows)} 行")

        # 测试 6: 查询 products 表
        print("\n    1.6 测试 SELECT * FROM products:")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="example",
            sql="SELECT * FROM products"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, f"查询失败: {sql_resp.message}"
        print(f"        ✓ 查询成功，返回 {len(sql_resp.rows)} 行")

        # ========== JOIN SQL 查询测试 ==========
        print("\n" + "-" * 50)
        print("【JOIN SQL 查询测试】")
        print("-" * 50)

        # 测试 7: INNER JOIN orders 和 customers
        print("\n    2.1 测试 INNER JOIN (orders + customers):")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="example",
            sql="SELECT o.id as order_id, o.customer_id, c.name as customer_name, o.total_amount "
                "FROM orders o INNER JOIN customers c ON o.customer_id = c.id "
                "LIMIT 5"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, f"INNER JOIN 查询失败: {sql_resp.message}"
        print(f"        ✓ 查询成功，返回 {len(sql_resp.rows)} 行")
        if len(sql_resp.rows) > 0:
            print(f"        示例数据: {sql_resp.rows[0]}")

        # 测试 8: LEFT JOIN orders 和 customers
        print("\n    2.2 测试 LEFT JOIN (orders + customers):")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="example",
            sql="SELECT o.id as order_id, c.name as customer_name, o.total_amount "
                "FROM orders o LEFT JOIN customers c ON o.customer_id = c.id "
                "LIMIT 5"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, f"LEFT JOIN 查询失败: {sql_resp.message}"
        print(f"        ✓ 查询成功，返回 {len(sql_resp.rows)} 行")

        # 测试 9: 多表 JOIN (orders, customers, products via order_items)
        print("\n    2.3 测试 order_items 表查询:")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="example",
            sql="SELECT * FROM order_items LIMIT 5"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, f"order_items 查询失败: {sql_resp.message}"
        print(f"        ✓ 查询成功，返回 {len(sql_resp.rows)} 行")
        if len(sql_resp.rows) > 0:
            print(f"        示例数据: {sql_resp.rows[0]}")

        # 测试 10: 带 WHERE 条件的 JOIN
        print("\n    2.4 测试带 WHERE 条件的 JOIN:")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="example",
            sql="SELECT o.id, c.name, o.total_amount "
                "FROM orders o INNER JOIN customers c ON o.customer_id = c.id "
                "WHERE o.total_amount > 50000 "
                "LIMIT 5"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, f"带 WHERE 的 JOIN 查询失败: {sql_resp.message}"
        print(f"        ✓ 查询成功，返回 {len(sql_resp.rows)} 行")

        # 测试 11: 聚合 + JOIN
        print("\n    2.5 测试聚合函数 + JOIN:")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="example",
            sql="SELECT c.name, COUNT(o.id) as order_count, SUM(o.total_amount) as total "
                "FROM customers c LEFT JOIN orders o ON c.id = o.customer_id "
                "GROUP BY c.name "
                "HAVING COUNT(o.id) > 0 "
                "LIMIT 5"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, f"聚合 + JOIN 查询失败: {sql_resp.message}"
        print(f"        ✓ 查询成功，返回 {len(sql_resp.rows)} 行")
        if len(sql_resp.rows) > 0:
            print(f"        示例数据: {sql_resp.rows[0]}")

        print("\n" + "=" * 70)
        print("SUCCESS! example schema SQL 查询测试全部通过")
        print("=" * 70)

        return 0

    except Exception as e:
        print(f"\n    ✗ 测试失败: {type(e).__name__}: {e}")
        import traceback
        traceback.print_exc()
        return 1
    finally:
        print("\n--- 终止服务进程 ---")
        try:
            os.killpg(os.getpgid(server_proc.pid), signal.SIGTERM)
            server_proc.wait(timeout=3)
        except:
            try:
                server_proc.kill()
            except:
                pass


if __name__ == "__main__":
    sys.exit(run_example_schema_sql_test())
