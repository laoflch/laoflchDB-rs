#!/usr/bin/env python3
"""
调试 JOIN 查询问题
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
    field = field_pb2.Field()
    if field_type == 0:
        field.string_value.value = value
    elif field_type == 1:
        field.integer_value.value = int(value)
    elif field_type == 3:
        field.float_value.value = float(value)
    return field.SerializeToString()

def run_debug():
    channel = grpc.insecure_channel("127.0.0.1:19777")
    stub = rpc_pb2_grpc.LaoflchDbStub(channel)
    
    print("\n=== 查询 customers 表 ===")
    sql_req = rpc_pb2.SqlQueryRequest(schema="sys", sql="SELECT * FROM customers")
    sql_resp = stub.SqlQuery(sql_req)
    print(f"成功: {sql_resp.success}")
    print(f"列: {sql_resp.columns}")
    print(f"行数: {len(sql_resp.rows)}")
    for row in sql_resp.rows:
        values = []
        for val in row.values:
            if val.HasField('int64_value'):
                values.append(str(val.int64_value))
            elif val.HasField('string_value'):
                values.append(f"'{val.string_value}'")
            elif val.HasField('float_value'):
                values.append(str(val.float_value))
        print(f"  {values}")
    
    print("\n=== 查询 orders 表 ===")
    sql_req = rpc_pb2.SqlQueryRequest(schema="sys", sql="SELECT * FROM orders")
    sql_resp = stub.SqlQuery(sql_req)
    print(f"成功: {sql_resp.success}")
    print(f"列: {sql_resp.columns}")
    print(f"行数: {len(sql_resp.rows)}")
    for row in sql_resp.rows:
        values = []
        for val in row.values:
            if val.HasField('int64_value'):
                values.append(str(val.int64_value))
            elif val.HasField('float_value'):
                values.append(str(val.float_value))
        print(f"  {values}")
    
    print("\n=== 测试 JOIN ===")
    sql_req = rpc_pb2.SqlQueryRequest(
        schema="sys",
        sql="SELECT c.name, o.order_id FROM customers c INNER JOIN orders o ON c.customer_id = o.customer_id"
    )
    sql_resp = stub.SqlQuery(sql_req)
    print(f"成功: {sql_resp.success}")
    print(f"行数: {len(sql_resp.rows)}")
    for row in sql_resp.rows:
        values = []
        for val in row.values:
            if val.HasField('int64_value'):
                values.append(str(val.int64_value))
            elif val.HasField('string_value'):
                values.append(f"'{val.string_value}'")
        print(f"  {values}")

if __name__ == "__main__":
    run_debug()