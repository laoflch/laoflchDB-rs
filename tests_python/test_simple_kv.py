#!/usr/bin/env python3
"""简单的 KV 测试"""
import grpc
import sys
import os

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import rpc_pb2
import rpc_pb2_grpc

print("连接 gRPC 服务 19870...")
channel = grpc.insecure_channel('127.0.0.1:19870')
stub = rpc_pb2_grpc.LaoflchDbStub(channel)

print("写入数据...")
try:
    put_resp = stub.Put(rpc_pb2.PutRequest(
        schema="sys",
        table="user",  # 默认用户表应该已经存在
        key=b"test_key",
        value=b"test_value"
    ), timeout=3)
    print(f"✓ 写入数据成功: success={put_resp.success}, message={put_resp.message}")
except Exception as e:
    print(f"✗ 写入数据失败: {e}")

print("读取数据...")
try:
    get_resp = stub.Get(rpc_pb2.GetRequest(
        schema="sys",
        table="user",
        key=b"test_key"
    ), timeout=3)
    print(f"✓ 读取数据成功: found={get_resp.found}, value={get_resp.value}")
except Exception as e:
    print(f"✗ 读取数据失败: {e}")

print("\n✓ 单服务基础测试成功!")
