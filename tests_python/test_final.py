#!/usr/bin/env python3
import sys
import os
import grpc

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import rpc_pb2
import rpc_pb2_grpc

SCHEMA = "sys"
TABLE_NAME = "test_grpc_api"

def run_tests():
    print("=" * 60)
    print("Python 自动回归测试: gRPC API 端到端验证")
    print("=" * 60)
    print()

    channel = grpc.insecure_channel("127.0.0.1:29777")
    stub = rpc_pb2_grpc.LaoflchDbStub(channel)

    tests = [
        ("创建表", test_create_table, stub),
        ("列出表", test_list_tables, stub),
        ("获取表元数据", test_get_table_meta, stub),
        ("插入数据", test_put_data, stub),
        ("读取数据", test_get_data, stub),
        ("更新数据", test_update_data, stub),
        ("查询数据", test_query_data, stub),
        ("删除数据", test_delete_data, stub),
        ("验证删除", test_verify_delete, stub),
        ("错误处理", test_error_handling, stub),
    ]

    passed = 0
    failed = 0

    for name, test_func, stub in tests:
        print(f"[测试] {name}...")
        try:
            if test_func(stub):
                print(f"    ✓ {name}通过")
                passed += 1
            else:
                print(f"    ✗ {name}失败")
                failed += 1
        except Exception as e:
            print(f"    ✗ {name}异常: {e}")
            failed += 1
        print()

    print("[清理] 删除测试表...")
    try:
        stub.DropTable(rpc_pb2.DropTableRequest(schema=SCHEMA, table_name=TABLE_NAME))
        print("    ✓ 清理完成")
    except:
        pass

    print("=" * 60)
    print(f"测试结果: {passed} 通过, {failed} 失败")
    print("=" * 60)

    if failed > 0:
        sys.exit(1)
    else:
        print("✓ 所有 gRPC API 测试通过！")
        sys.exit(0)

def test_create_table(stub):
    resp = stub.CreateTable(rpc_pb2.CreateTableRequest(
        schema=SCHEMA,
        table_name=TABLE_NAME,
        columns=[
            rpc_pb2.ColumnDef(name="id", column_type=1),
            rpc_pb2.ColumnDef(name="name", column_type=2),
            rpc_pb2.ColumnDef(name="email", column_type=2),
            rpc_pb2.ColumnDef(name="age", column_type=1),
        ]
    ))
    return resp.success

def test_list_tables(stub):
    resp = stub.ListTables(rpc_pb2.ListTablesRequest(schema=SCHEMA))
    return resp.success and TABLE_NAME in resp.tables

def test_get_table_meta(stub):
    resp = stub.GetTableMeta(rpc_pb2.GetTableMetaRequest(schema=SCHEMA, table_name=TABLE_NAME))
    return resp.success and resp.table_name == TABLE_NAME

def test_put_data(stub):
    test_data = [
        (b"user_001", b'{"id":1,"name":"Alice","email":"alice@example.com","age":25}'),
        (b"user_002", b'{"id":2,"name":"Bob","email":"bob@example.com","age":30}'),
        (b"user_003", b'{"id":3,"name":"Charlie","email":"charlie@example.com","age":35}'),
    ]
    for key, value in test_data:
        resp = stub.Put(rpc_pb2.PutRequest(schema=SCHEMA, table=TABLE_NAME, key=key, value=value))
        if not resp.success:
            return False
    return True

def test_get_data(stub):
    resp = stub.Get(rpc_pb2.GetRequest(schema=SCHEMA, table=TABLE_NAME, key=b"user_001"))
    return resp.success and resp.value is not None

def test_update_data(stub):
    new_value = b'{"id":1,"name":"Alice Updated","email":"alice.updated@example.com","age":26}'
    resp = stub.Put(rpc_pb2.PutRequest(schema=SCHEMA, table=TABLE_NAME, key=b"user_001", value=new_value))
    if not resp.success:
        return False
    
    resp = stub.Get(rpc_pb2.GetRequest(schema=SCHEMA, table=TABLE_NAME, key=b"user_001"))
    return resp.success and resp.value == new_value

def test_query_data(stub):
    resp = stub.Query(rpc_pb2.QueryRequest(
        schema=SCHEMA,
        table_filters=[
            rpc_pb2.TableFilter(
                table_name=TABLE_NAME,
                column_filters=[
                    rpc_pb2.ColumnFilter(
                        column_name="age",
                        conditions=[
                            rpc_pb2.ColumnFilterCondition(
                                op=rpc_pb2.FILTER_OPERATOR_GT,
                                value=rpc_pb2.Field(integer_value=rpc_pb2.IntegerValue(value=25))
                            )
                        ]
                    )
                ]
            )
        ],
        limit=10,
        offset=0
    ))
    return resp.success and len(resp.rows) >= 0

def test_delete_data(stub):
    resp = stub.Delete(rpc_pb2.DeleteRequest(schema=SCHEMA, table=TABLE_NAME, key=b"user_001"))
    return resp.success

def test_verify_delete(stub):
    resp = stub.Get(rpc_pb2.GetRequest(schema=SCHEMA, table=TABLE_NAME, key=b"user_001"))
    return resp.success and (resp.value is None or resp.value == b"")

def test_error_handling(stub):
    try:
        resp = stub.Get(rpc_pb2.GetRequest(schema=SCHEMA, table="nonexistent_table", key=b"test"))
        return not resp.success
    except grpc.RpcError as e:
        return True

if __name__ == "__main__":
    run_tests()