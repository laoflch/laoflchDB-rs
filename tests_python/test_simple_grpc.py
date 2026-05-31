#!/usr/bin/env python3
"""简单的测试脚本"""
import grpc
import sys
import os

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import rpc_pb2
import rpc_pb2_grpc

print("连接 gRPC 服务 19777...")
channel = grpc.insecure_channel('127.0.0.1:19777')
stub = rpc_pb2_grpc.LaoflchDbStub(channel)

print("创建表...")
req = rpc_pb2.CreateTableRequest(
    schema="sys",
    table_name="test_table",
    columns=[
        rpc_pb2.ColumnDef(name="id", column_type=2),
    ]
)
try:
    resp = stub.CreateTable(req, timeout=3)
    print(f"✓ 创建表成功: {resp}")
except Exception as e:
    print(f"✗ 创建表失败: {e}")
    sys.exit(1)

print("写入数据...")
try:
    put_resp = stub.Put(rpc_pb2.PutRequest(
        schema="sys",
        table="test_table",
        key=b"key1",
        value=b"value1"
    ), timeout=3)
    print(f"✓ 写入数据成功: {put_resp}")
except Exception as e:
    print(f"✗ 写入数据失败: {e}")

print("读取数据...")
try:
    get_resp = stub.Get(rpc_pb2.GetRequest(
        schema="sys",
        table="test_table",
        key=b"key1"
    ), timeout=3)
    print(f"✓ 读取数据成功: success={get_resp.success}, value={get_resp.value}")
except Exception as e:
    print(f"✗ 读取数据失败: {e}")

print("\n✓ 单服务测试成功!")
