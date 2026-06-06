#!/usr/bin/env python3
"""简单的 gRPC SQL 查询测试"""
import grpc
import time
import sys

sys.path.insert(0, "/workspace/rust_space/laoflchDB-rust/tests_python")

import rpc_pb2
import rpc_pb2_grpc

def test_simple_sql():
    print("测试 gRPC SQL 查询...")
    
    channel = grpc.insecure_channel("127.0.0.1:19777")
    stub = rpc_pb2_grpc.LaoflchDbStub(channel)
    
    try:
        # 先尝试删除旧表
        try:
            stub.DropTable(rpc_pb2.DropTableRequest(schema="sys", table_name="test_simple"))
            print("已删除旧表")
        except:
            pass
        
        # 创建表（包含 float 列）
        create_req = rpc_pb2.CreateTableRequest(
            schema="sys",
            table_name="test_simple",
            columns=[
                rpc_pb2.ColumnDef(name="id", column_type=1),      # INT64
                rpc_pb2.ColumnDef(name="name", column_type=2),    # STRING
                rpc_pb2.ColumnDef(name="age", column_type=1),     # INT64
                rpc_pb2.ColumnDef(name="score", column_type=4),   # FLOAT
            ]
        )
        create_resp = stub.CreateTable(create_req)
        print(f"创建表成功: {create_resp.success}")
        
        time.sleep(1)
        
        # 插入数据（包含 float 值）
        add_req = rpc_pb2.AddRowRequest(
            schema="sys",
            table_name="test_simple",
            row=rpc_pb2.Row(
                row_type=0,
                version=1,
                data=[
                    str(1).encode('utf-8'),
                    "Alice".encode('utf-8'),
                    str(30).encode('utf-8'),
                    str(95.5).encode('utf-8'),
                ]
            )
        )
        add_resp = stub.AddRow(add_req)
        print(f"插入数据成功: {add_resp.success}")
        
        time.sleep(0.5)
        
        # 测试 SQL 查询
        print("\n执行 SQL 查询: SELECT * FROM test_simple")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT * FROM test_simple"
        )
        
        try:
            sql_resp = stub.SqlQuery(sql_req, timeout=30)
            print(f"查询成功: {sql_resp.success}")
            print(f"返回行数: {len(sql_resp.rows)}")
            print(f"列名: {sql_resp.columns}")
        except grpc.RpcError as e:
            print(f"查询失败: {e.code()} - {e.details()}")
            import traceback
            traceback.print_exc()
            
    finally:
        channel.close()

if __name__ == "__main__":
    test_simple_sql()
