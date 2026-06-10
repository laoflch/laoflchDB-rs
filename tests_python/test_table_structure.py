#!/usr/bin/env python3
"""简单测试：检查表结构是否正确注册"""
import grpc
import sys

sys.path.insert(0, '/workspace/rust_space/laoflchDB-rust/tests_python')

import rpc_pb2
import rpc_pb2_grpc

def test_table_structure():
    channel = grpc.insecure_channel('127.0.0.1:19777')
    stub = rpc_pb2_grpc.LaoflchDbStub(channel)
    
    # 先创建一个测试表
    print("创建测试表...")
    try:
        stub.DropTable(rpc_pb2.DropTableRequest(
            schema="test_schema",
            table_name="test_table"
        ))
    except:
        pass
    
    stub.CreateTable(rpc_pb2.CreateTableRequest(
        schema="test_schema",
        table_name="test_table",
        columns=[
            rpc_pb2.ColumnDef(name="id", column_type=1),       # INT64
            rpc_pb2.ColumnDef(name="name", column_type=0),     # STRING
            rpc_pb2.ColumnDef(name="value", column_type=3),    # FLOAT
        ]
    ))
    print("表创建成功")
    
    # 查询列信息
    print("\n查询表列信息:")
    resp = stub.ListTableCols(rpc_pb2.ListTableColsRequest(
        schema="test_schema",
        table_name="test_table"
    ))
    print(f"列数量: {len(resp.columns)}")
    for col in resp.columns:
        print(f"  - {col.column_name} (type: {col.column_type})")
    
    # 执行 SQL 查询
    print("\n执行 SQL 查询:")
    try:
        resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(
            schema="test_schema",
            sql="SELECT * FROM test_schema.test_table"
        ))
        print(f"查询成功，返回 {len(resp.rows)} 行")
        print(f"列名: {resp.columns}")
        
        # 打印行数据
        print(f"\n行数据:")
        for row_idx, row in enumerate(resp.rows):
            print(f"  行 {row_idx}:")
            for col_idx, field in enumerate(row.values):
                val = None
                if field.HasField('string_value'):
                    val = f"string: {field.string_value}"
                elif field.HasField('int64_value'):
                    val = f"int64: {field.int64_value}"
                elif field.HasField('float_value'):
                    val = f"float: {field.float_value}"
                elif field.HasField('bytes_value'):
                    val = f"bytes: {field.bytes_value}"
                print(f"    字段 {col_idx}: {val}")
    except Exception as e:
        print(f"查询失败: {e}")
    
    channel.close()

if __name__ == "__main__":
    test_table_structure()
