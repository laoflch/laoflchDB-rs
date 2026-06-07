#!/usr/bin/env python3
"""
详细调试 JOIN 查询问题
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
    # 启动服务
    TEST_DB = "./laoflch_db_debug"
    TEST_ADDR = "127.0.0.1:19777"
    SERVER_BIN = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "release", "laoflchDB-rust")
    
    subprocess.run(["rm", "-rf", TEST_DB], capture_output=True)
    subprocess.run([SERVER_BIN, "init", "--db-path", TEST_DB], capture_output=True)
    
    server_proc = subprocess.Popen(
        [SERVER_BIN, "start", "--addr", TEST_ADDR, "--db-path", TEST_DB],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True
    )
    
    time.sleep(4)
    
    try:
        channel = grpc.insecure_channel(TEST_ADDR)
        stub = rpc_pb2_grpc.LaoflchDbStub(channel)
        
        # 创建表
        print("\n=== 创建表 ===")
        stub.CreateTable(rpc_pb2.CreateTableRequest(
            schema="sys",
            table_name="customers",
            columns=[
                rpc_pb2.ColumnDef(name="customer_id", column_type=1),
                rpc_pb2.ColumnDef(name="name", column_type=0),
                rpc_pb2.ColumnDef(name="city", column_type=0),
            ]
        ))
        print("✓ 创建 customers 表")
        
        stub.CreateTable(rpc_pb2.CreateTableRequest(
            schema="sys",
            table_name="orders",
            columns=[
                rpc_pb2.ColumnDef(name="order_id", column_type=1),
                rpc_pb2.ColumnDef(name="customer_id", column_type=1),
                rpc_pb2.ColumnDef(name="amount", column_type=3),
            ]
        ))
        print("✓ 创建 orders 表")
        
        time.sleep(3)
        
        # 插入数据
        print("\n=== 插入数据 ===")
        
        # customers
        customers = [(1, "Alice", "NY"), (2, "Bob", "London")]
        for cid, name, city in customers:
            stub.AddRow(rpc_pb2.AddRowRequest(
                schema="sys",
                table_name="customers",
                row=rpc_pb2.Row(
                    row_type=0,
                    version=1,
                    data=[encode_field(cid, 1), encode_field(name, 0), encode_field(city, 0)]
                )
            ))
            print(f"✓ 插入 customer {cid}: {name}")
        
        # orders
        orders = [(101, 1, 100.0), (102, 1, 200.0), (103, 2, 150.0)]
        for oid, cid, amount in orders:
            stub.AddRow(rpc_pb2.AddRowRequest(
                schema="sys",
                table_name="orders",
                row=rpc_pb2.Row(
                    row_type=0,
                    version=1,
                    data=[encode_field(oid, 1), encode_field(cid, 1), encode_field(amount, 3)]
                )
            ))
            print(f"✓ 插入 order {oid}: customer={cid}, amount={amount}")
        
        time.sleep(1)
        
        # 查询单表
        print("\n=== 查询 customers ===")
        resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(schema="sys", sql="SELECT * FROM customers"))
        print(f"成功: {resp.success}, 行数: {len(resp.rows)}")
        for row in resp.rows:
            vals = []
            for v in row.values:
                if v.HasField('int64_value'):
                    vals.append(str(v.int64_value))
                elif v.HasField('string_value'):
                    vals.append(f"'{v.string_value}'")
            print(f"  {vals}")
        
        print("\n=== 查询 orders ===")
        resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(schema="sys", sql="SELECT * FROM orders"))
        print(f"成功: {resp.success}, 行数: {len(resp.rows)}")
        for row in resp.rows:
            vals = []
            for v in row.values:
                if v.HasField('int64_value'):
                    vals.append(str(v.int64_value))
                elif v.HasField('float_value'):
                    vals.append(str(v.float_value))
            print(f"  {vals}")
        
        # JOIN 查询
        print("\n=== JOIN 查询 ===")
        sql = "SELECT c.name, o.order_id FROM customers c, orders o WHERE c.customer_id = o.customer_id"
        print(f"SQL: {sql}")
        resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(schema="sys", sql=sql))
        print(f"成功: {resp.success}, 行数: {len(resp.rows)}")
        for row in resp.rows:
            vals = []
            for v in row.values:
                if v.HasField('int64_value'):
                    vals.append(str(v.int64_value))
                elif v.HasField('string_value'):
                    vals.append(f"'{v.string_value}'")
            print(f"  {vals}")
        
    finally:
        try:
            os.killpg(os.getpgid(server_proc.pid), signal.SIGTERM)
        except:
            try:
                server_proc.kill()
            except:
                pass

if __name__ == "__main__":
    run_debug()