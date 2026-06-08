#!/usr/bin/env python3
"""
gRPC SQL JOIN 查询测试 - 验证表连接功能
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

TEST_DB = "./laoflch_db_grpc_join_test"
TEST_ADDR = "127.0.0.1:19777"
SERVER_BIN = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "release", "laoflchdb")

def run_grpc_join_test():
    os.chdir(os.path.dirname(os.path.abspath(__file__)))

    print("=" * 70)
    print("Python 自动回归测试: gRPC SQL JOIN 查询")
    print("=" * 70)

    print("\n[1/7] 编译 Rust release 版本...")
    result = subprocess.run(["cargo", "build", "--release"], cwd="..", capture_output=True)
    if result.returncode != 0:
        print("编译失败:", result.stderr.decode())
        return 1
    print("    ✓ 编译完成")

    print("\n[2/7] 清理旧测试数据...")
    subprocess.run(["rm", "-rf", TEST_DB], capture_output=True)
    print("    ✓ 清理完成")

    print("\n[3/7] 启动 laoflchDB gRPC 服务后台进程...")
    cmd = [
        SERVER_BIN,
        "start",
        "--addr", TEST_ADDR,
        "--db-path", TEST_DB
    ]
    server_proc = subprocess.Popen(
        cmd,
        cwd="..",
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True
    )
    time.sleep(4)
    print(f"    ✓ 服务已启动 PID={server_proc.pid} 监听 {TEST_ADDR}")

    try:
        print("\n[4/7] 等待服务就绪...")
        max_retries = 10
        token = None
        metadata = []
        for i in range(max_retries):
            try:
                channel = grpc.insecure_channel(TEST_ADDR)
                stub = rpc_pb2_grpc.LaoflchDbStub(channel)
                login_resp = stub.Login(rpc_pb2.LoginRequest(
                    username="admin",
                    password="admin123"
                ))
                if login_resp.success:
                    token = login_resp.token
                    metadata = [('authorization', f'Bearer {token}')]
                    print(f"    ✓ 服务已就绪并登录成功 (尝试 {i+1}/{max_retries})")
                    break
            except grpc.RpcError as e:
                print(f"    服务尚未就绪 (尝试 {i+1}/{max_retries}): {e.code()}")
                time.sleep(1)
                continue
        
        print("\n[5/7] 创建测试表...")
        
        # 删除旧表
        for table_name in ["orders", "customers"]:
            try:
                drop_req = rpc_pb2.DropTableRequest(
                    schema="sys",
                    table_name=table_name
                )
                stub.DropTable(drop_req, metadata=metadata)
                print(f"    - 已删除旧表 {table_name}")
            except grpc.RpcError as e:
                if e.code() == grpc.StatusCode.NOT_FOUND:
                    print(f"    - 表 {table_name} 不存在，跳过删除")
        
        # 创建 customers 表
        create_customer_req = rpc_pb2.CreateTableRequest(
            schema="sys",
            table_name="customers",
            columns=[
                rpc_pb2.ColumnDef(name="customer_id", column_type=1),  # INT64
                rpc_pb2.ColumnDef(name="name", column_type=0),        # STRING
                rpc_pb2.ColumnDef(name="city", column_type=0),        # STRING
            ]
        )
        create_resp = stub.CreateTable(create_customer_req, metadata=metadata)
        assert create_resp.success == True, "创建 customers 表失败"
        print("    ✓ 创建 customers 表成功")
        
        # 创建 orders 表
        create_order_req = rpc_pb2.CreateTableRequest(
            schema="sys",
            table_name="orders",
            columns=[
                rpc_pb2.ColumnDef(name="order_id", column_type=1),    # INT64
                rpc_pb2.ColumnDef(name="customer_id", column_type=1), # INT64
                rpc_pb2.ColumnDef(name="amount", column_type=3),      # FLOAT
            ]
        )
        create_resp = stub.CreateTable(create_order_req, metadata=metadata)
        assert create_resp.success == True, "创建 orders 表失败"
        print("    ✓ 创建 orders 表成功")
        
        # 等待表注册到 SQL 引擎（sys schema 的表会自动注册）
        print("    等待表注册到 SQL 引擎...")
        time.sleep(2)
        
        print("\n[6/7] 插入测试数据...")
        
        # 插入 customers 数据
        customers_data = [
            (1, "Alice", "New York"),
            (2, "Bob", "London"),
            (3, "Charlie", "Paris"),
            (4, "David", "Tokyo"),
        ]
        
        for customer_id, name, city in customers_data:
            add_req = rpc_pb2.AddRowRequest(
                schema="sys",
                table_name="customers",
                row=rpc_pb2.Row(
                    row_type=0,
                    version=1,
                    data=[
                        encode_field(customer_id, 1),  # customer_id
                        encode_field(name, 0),         # name
                        encode_field(city, 0),         # city
                    ]
                )
            )
            add_resp = stub.AddRow(add_req, metadata=metadata)
            assert add_resp.success == True, f"插入 customer {customer_id} 失败"
            print(f"    ✓ 插入 customer {customer_id}: {name}")
        
        # 插入 orders 数据
        orders_data = [
            (101, 1, 100.50),   # Alice 的订单
            (102, 1, 200.75),   # Alice 的订单
            (103, 2, 150.00),   # Bob 的订单
            (104, 3, 75.25),    # Charlie 的订单
            (105, 5, 300.00),   # 不存在的 customer_id=5
        ]
        
        for order_id, customer_id, amount in orders_data:
            add_req = rpc_pb2.AddRowRequest(
                schema="sys",
                table_name="orders",
                row=rpc_pb2.Row(
                    row_type=0,
                    version=1,
                    data=[
                        encode_field(order_id, 1),    # order_id
                        encode_field(customer_id, 1), # customer_id
                        encode_field(amount, 3),      # amount
                    ]
                )
            )
            add_resp = stub.AddRow(add_req, metadata=metadata)
            assert add_resp.success == True, f"插入 order {order_id} 失败"
            print(f"    ✓ 插入 order {order_id}: customer={customer_id}, amount={amount}")
        
        time.sleep(1)
        
        print("\n[7/7] 测试 JOIN 查询...")
        
        # 检查表注册状态
        print("    检查表注册状态...")
        list_resp = stub.ListTables(rpc_pb2.ListTablesRequest(schema="sys"), metadata=metadata)
        print(f"    当前表列表: {list_resp.tables}")
        
        time.sleep(2)
        
        # 测试 INNER JOIN
        print("\n    测试 INNER JOIN:")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT c.name, o.order_id, o.amount FROM customers c INNER JOIN orders o ON c.customer_id = o.customer_id"
        )
        sql_resp = stub.SqlQuery(sql_req, metadata=metadata)
        assert sql_resp.success == True, "INNER JOIN 查询失败"
        print(f"        ✓ INNER JOIN 查询成功，返回 {len(sql_resp.rows)} 行")
        
        # 测试 LEFT JOIN
        print("\n    测试 LEFT JOIN:")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT c.name, o.order_id, o.amount FROM customers c LEFT JOIN orders o ON c.customer_id = o.customer_id"
        )
        sql_resp = stub.SqlQuery(sql_req, metadata=metadata)
        assert sql_resp.success == True, "LEFT JOIN 查询失败"
        print(f"        ✓ LEFT JOIN 查询成功，返回 {len(sql_resp.rows)} 行")
        
        # 测试 RIGHT JOIN
        print("\n    测试 RIGHT JOIN:")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT c.name, o.order_id, o.amount FROM customers c RIGHT JOIN orders o ON c.customer_id = o.customer_id"
        )
        sql_resp = stub.SqlQuery(sql_req, metadata=metadata)
        assert sql_resp.success == True, "RIGHT JOIN 查询失败"
        print(f"        ✓ RIGHT JOIN 查询成功，返回 {len(sql_resp.rows)} 行")
        
        # 测试带 WHERE 条件的 JOIN
        print("\n    测试带 WHERE 条件的 JOIN:")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT c.name, o.order_id, o.amount FROM customers c INNER JOIN orders o ON c.customer_id = o.customer_id WHERE o.amount > 100"
        )
        sql_resp = stub.SqlQuery(sql_req, metadata=metadata)
        assert sql_resp.success == True, "带 WHERE 条件的 JOIN 查询失败"
        print(f"        ✓ 带 WHERE 条件的 JOIN 查询成功，返回 {len(sql_resp.rows)} 行")
        
        # 测试聚合函数 + JOIN
        print("\n    测试聚合函数 + JOIN:")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT c.name, COUNT(o.order_id) as order_count FROM customers c LEFT JOIN orders o ON c.customer_id = o.customer_id GROUP BY c.name"
        )
        sql_resp = stub.SqlQuery(sql_req, metadata=metadata)
        assert sql_resp.success == True, "聚合函数 + JOIN 查询失败"
        print(f"        ✓ 聚合函数 + JOIN 查询成功，返回 {len(sql_resp.rows)} 行")
        
        print("\n" + "=" * 70)
        print("SUCCESS! gRPC SQL JOIN 查询测试全部通过")
        print("=" * 70)
        print(f"数据保留在: {TEST_DB}")

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
        print(f"数据保留在: {TEST_DB}")

if __name__ == "__main__":
    sys.exit(run_grpc_join_test())