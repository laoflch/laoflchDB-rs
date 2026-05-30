#!/usr/bin/env python3
import sys
import os
import grpc

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import rpc_pb2
import rpc_pb2_grpc

print("=== 通过 gRPC 写入测试数据 ===")
test_data = [
    (b"grpc_user_001", b'{"user_id": 2001, "password": "grpc_pass_001"}'),
    (b"grpc_user_002", b'{"user_id": 2002, "password": "grpc_pass_002"}'),
    (b"grpc_user_003", b'{"user_id": 2003, "password": "grpc_pass_003"}'),
]

channel = grpc.insecure_channel("127.0.0.1:19777")
stub = rpc_pb2_grpc.LaoflchDbStub(channel)

for key, value in test_data:
    print(f"写入: key={key.decode()}")
    print(f"      value={value.decode()}")
    stub.Put(rpc_pb2.PutRequest(table="user", key=key, value=value))
    print(f"      ✓ 成功")

print("\n=== 通过 gRPC 读取验证 ===")
for key, expected in test_data:
    resp = stub.Get(rpc_pb2.GetRequest(table="user", key=key))
    print(f"读取: key={key.decode()}")
    print(f"      found={resp.found}, value={resp.value.decode()}")
    assert resp.found
    assert resp.value == expected
    print(f"      ✓ 验证通过")

print("\n=== 测试完成！数据保留在 laoflch_db_data ===")
