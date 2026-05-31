#!/usr/bin/env python3
import time
import sys
import os

# Add the current directory to the Python path
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import grpc
import rpc_pb2
import rpc_pb2_grpc

TEST_ADDR = "127.0.0.1:19777"


def print_result(call_name, message, indent=2):
    print(f"\n>>> [gRPC 结果] {call_name}: ")
    for line in str(message).split('\n'):
        if line.strip():
            print(f"{' '*indent}{line}")


def main():
    print("=" * 70)
    print("Python gRPC Query 接口自动测试")
    print("=" * 70)

    print("\n--- 连接到服务 ---")
    time.sleep(1)

    try:
        channel = grpc.insecure_channel(TEST_ADDR)
        stub = rpc_pb2_grpc.LaoflchDbStub(channel)

        # 1. 创建表
        print("\n--- 1. 创建测试表 ---")
        create_resp = stub.CreateTable(rpc_pb2.CreateTableRequest(
            schema="sys",
            table_name="query_test_table",
            columns=[
                rpc_pb2.ColumnDef(name="id", column_type=1),  # 假设 1 是整数类型
                rpc_pb2.ColumnDef(name="name", column_type=2)  # 假设 2 是字符串类型
            ]
        ), timeout=3)
        print_result("CreateTable", create_resp)
        assert create_resp.success

        # 2. 写入 KV 数据（简单的方式先测试 Query 接口）
        print("\n--- 2. 写入 KV 数据 ---")
        for i in range(5):
            put_resp = stub.Put(rpc_pb2.PutRequest(
                schema="sys",
                table="query_test_table",
                key=f"key_{i}".encode(),
                value=f"value_{i}".encode()
            ), timeout=3)
            print(f"  Put key_{i}: {'✓' if put_resp.success else '✗'}")
            assert put_resp.success

        # 3. 测试无过滤的 Query
        print("\n--- 3. 测试无过滤条件的 Query ---")
        query_resp = stub.Query(rpc_pb2.QueryRequest(
            schema="sys",
            limit=10
        ), timeout=3)
        print_result("Query (no filters)", query_resp)
        assert query_resp.success

        print("\n✅ 所有 Query 接口基础测试通过!")

        # 清理
        print("\n--- 清理 ---")
        drop_resp = stub.DropTable(rpc_pb2.DropTableRequest(
            schema="sys",
            table_name="query_test_table"
        ), timeout=3)
        print(f"  Table dropped: {'✓' if drop_resp.success else '✗'}")

    except Exception as e:
        print(f"\n❌ ERROR: {e}")
        import traceback
        traceback.print_exc()
        raise


if __name__ == "__main__":
    main()

