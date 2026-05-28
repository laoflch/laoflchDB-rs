#!/usr/bin/env python3
import subprocess
import time
import sys
import os
import signal
import grpc

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import rpc_pb2
import rpc_pb2_grpc

TEST_DB = "./test_detailed_db"
TEST_ADDR = "127.0.0.1:19999"

def print_result(call_name, message, indent=2):
    print(f"\n>>> [gRPC 结果] {call_name}: ")
    for line in message.split('\n'):
        if line.strip():
            print(f"{' '*indent}{line}")

def main():
    os.chdir(os.path.dirname(os.path.abspath(__file__)))

    print("=" * 70)
    print("Python gRPC 自动回归测试：详细打印所有查询结果")
    print("=" * 70)

    print("\n--- 清理并启动服务 ---")
    subprocess.run(["rm", "-rf", TEST_DB], cwd="..")
    server_proc = subprocess.Popen(
        ["../target/release/laoflchDB-rust", "start", "--addr", TEST_ADDR, "--db-path", TEST_DB],
        cwd="..", stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, preexec_fn=os.setsid
    )
    time.sleep(2)
    print(f"服务 PID {server_proc.pid}")

    try:
        channel = grpc.insecure_channel(TEST_ADDR)
        stub = rpc_pb2_grpc.LaoflchDbStub(channel)

        # ============ 1. ListTables ============
        resp = stub.ListTables(rpc_pb2.ListTablesRequest())
        print_result("ListTables 查询结果", f"tables = {list(resp.tables)}")

        # ============ 2. CreateTable ============
        ct = stub.CreateTable(rpc_pb2.CreateTableRequest(
            table_name="orders",
            columns=[
                rpc_pb2.ColumnDef(name="order_id", col_type=1),
                rpc_pb2.ColumnDef(name="product_name", col_type=2),
            ]
        ))
        print_result("CreateTable 查询结果", f"新表 UUID = `{ct.table_id}`")

        # ============ 3. ListTables Again ============
        resp = stub.ListTables(rpc_pb2.ListTablesRequest())
        print_result("再次 ListTables 查询结果", f"tables = {list(resp.tables)}")

        # ============ 4. Put Data ============
        key1 = b"order_001_xiaomi"
        val1 = b'{"product": "phone", "quantity": 2, "price": 3999}'
        stub.Put(rpc_pb2.PutRequest(table="orders", key=key1, value=val1))
        print_result("Put 写入", f"表=orders, key={key1.decode()}, value_len={len(val1)} 已完成")

        key2 = b"order_002_huawei"
        val2 = b'{"product": "laptop", "quantity": 1, "price": 6999}'
        stub.Put(rpc_pb2.PutRequest(table="orders", key=key2, value=val2))
        print_result("Put 写入 2", f"表=orders, key={key2.decode()} 已完成")

        # ============ 5. Get Data Back ============
        get1 = stub.Get(rpc_pb2.GetRequest(table="orders", key=key1))
        print_result("Get 查询 order_001_xiaomi",
            f"found={get1.found}\nvalue_bytes={bytes(get1.value)}\nvalue_utf8={get1.value.decode('utf8')}")

        get2 = stub.Get(rpc_pb2.GetRequest(table="orders", key=key2))
        print_result("Get 查询 order_002_huawei",
            f"found={get2.found}\nvalue_bytes={bytes(get2.value)}\nvalue_utf8={get2.value.decode('utf8')}")

        # ============ 6. Not Found Test ============
        get3 = stub.Get(rpc_pb2.GetRequest(table="orders", key=b"notexist"))
        print_result("Get 查询不存在的 key", f"found={get3.found} (预期 false，结果正确)")

        assert get1.found == True
        assert get2.found == True
        assert get1.value == val1
        assert get2.value == val2
        assert get3.found == False

        print("\n" + "=" * 70)
        print("SUCCESS: 所有 gRPC 查询完成，结果全部正确")
        print("=" * 70)

    finally:
        os.killpg(os.getpgid(server_proc.pid), signal.SIGTERM)
        server_proc.wait()
        subprocess.run(["rm", "-rf", TEST_DB], cwd="..")

if __name__ == "__main__":
    main()
