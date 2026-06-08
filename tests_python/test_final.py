#!/usr/bin/env python3
import sys
import os
import grpc
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import rpc_pb2
import rpc_pb2_grpc
import field_pb2

def encode_field(value, field_type):
    """将值编码为 protobuf Field 对象"""
    field = field_pb2.Field()
    if field_type == 0:  # STRING
        field.string_value.value = value
    elif field_type == 1:  # INT64
        field.integer_value.value = int(value)
    elif field_type == 3:  # FLOAT
        field.float_value.value = float(value)
    elif field_type == 2:  # BYTES
        field.bytes_value.value = value if isinstance(value, bytes) else value.encode()
    return field.SerializeToString()

SCHEMA = "sys"
TABLE_NAME = "test_grpc_api"
SQL_TABLE_NAME = "test_sql_table"

def run_tests():
    print("=" * 60)
    print("Python 自动回归测试: gRPC API 端到端验证")
    print("=" * 60)
    print()

    import requests

    # 通过 REST API 登录获取 token
    print("[测试] 用户登录...")
    token = None
    try:
        resp = requests.post(
            "http://127.0.0.1:8080/api/v1/login",
            json={"username": "admin", "password": "laoflchdb"}
        )
        data = resp.json()
        if data.get("success") and data.get("data", {}).get("success"):
            token = data["data"]["token"]
            metadata = [('authorization', f'Bearer {token}'.encode())]
            print(f"    ✓ 登录成功，Token: {token[:20]}...")
        else:
            print(f"    ✗ 登录失败: {data}")
            return
    except Exception as e:
        print(f"    ✗ 登录异常: {e}")
        return

    channel = grpc.insecure_channel("127.0.0.1:19777")
    stub = rpc_pb2_grpc.LaoflchDbStub(channel)

    tests = [
        ("创建表", test_create_table, stub, metadata),
        ("列出表", test_list_tables, stub, metadata),
        ("获取表元数据", test_get_table_meta, stub, metadata),
        ("插入数据", test_put_data, stub, metadata),
        ("读取数据", test_get_data, stub, metadata),
        ("更新数据", test_update_data, stub, metadata),
        ("查询数据", test_query_data, stub, metadata),
        ("删除数据", test_delete_data, stub, metadata),
        ("验证删除", test_verify_delete, stub, metadata),
        ("错误处理", test_error_handling, stub, metadata),
        # SQL 查询全链路测试
        ("创建SQL测试表", test_create_sql_table, stub, metadata),
        ("添加SQL测试数据", test_add_sql_data, stub, metadata),
        ("SQL查询-SELECT", test_sql_query_select, stub, metadata),
        ("SQL查询-过滤", test_sql_query_filter, stub, metadata),
        ("SQL查询-聚合", test_sql_query_aggregate, stub, metadata),
        ("删除SQL测试表", test_drop_sql_table, stub, metadata),
    ]

    passed = 0
    failed = 0

    for name, test_func, stub, metadata in tests:
        print(f"[测试] {name}...")
        try:
            if test_func(stub, metadata):
                print(f"    ✓ {name}通过")
                passed += 1
            else:
                print(f"    ✗ {name}失败")
                failed += 1
        except Exception as e:
            print(f"    ✗ {name}异常: {e}")
            import traceback
            traceback.print_exc()
            failed += 1
        print()

    print("[清理] 删除测试表...")
    try:
        stub.DropTable(rpc_pb2.DropTableRequest(schema=SCHEMA, table_name=TABLE_NAME), metadata=metadata)
        print("    ✓ 清理完成")
    except:
        pass

    try:
        stub.DropTable(rpc_pb2.DropTableRequest(schema=SCHEMA, table_name=SQL_TABLE_NAME))
        print("    ✓ SQL测试表清理完成")
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

def test_create_table(stub, metadata):
    resp = stub.CreateTable(rpc_pb2.CreateTableRequest(
        schema=SCHEMA,
        table_name=TABLE_NAME,
        columns=[
            rpc_pb2.ColumnDef(name="id", column_type=1),
            rpc_pb2.ColumnDef(name="name", column_type=2),
            rpc_pb2.ColumnDef(name="email", column_type=2),
            rpc_pb2.ColumnDef(name="age", column_type=1),
        ]
    ), metadata=metadata)
    return resp.success

def test_list_tables(stub, metadata):
    resp = stub.ListTables(rpc_pb2.ListTablesRequest(schema=SCHEMA), metadata=metadata)
    return resp.success and TABLE_NAME in resp.tables

def test_get_table_meta(stub, metadata):
    resp = stub.GetTableMeta(rpc_pb2.GetTableMetaRequest(schema=SCHEMA, table_name=TABLE_NAME), metadata=metadata)
    return resp.success and resp.table_name == TABLE_NAME

def test_put_data(stub, metadata):
    test_data = [
        (b"user_001", b'{"id":1,"name":"Alice","email":"alice@example.com","age":25}'),
        (b"user_002", b'{"id":2,"name":"Bob","email":"bob@example.com","age":30}'),
        (b"user_003", b'{"id":3,"name":"Charlie","email":"charlie@example.com","age":35}'),
    ]
    for key, value in test_data:
        resp = stub.Put(rpc_pb2.PutRequest(schema=SCHEMA, table=TABLE_NAME, key=key, value=value), metadata=metadata)
        if not resp.success:
            return False
    return True

def test_get_data(stub, metadata):
    resp = stub.Get(rpc_pb2.GetRequest(schema=SCHEMA, table=TABLE_NAME, key=b"user_001"), metadata=metadata)
    return resp.success and resp.value is not None

def test_update_data(stub, metadata):
    new_value = b'{"id":1,"name":"Alice Updated","email":"alice.updated@example.com","age":26}'
    resp = stub.Put(rpc_pb2.PutRequest(schema=SCHEMA, table=TABLE_NAME, key=b"user_001", value=new_value), metadata=metadata)
    if not resp.success:
        return False
    
    resp = stub.Get(rpc_pb2.GetRequest(schema=SCHEMA, table=TABLE_NAME, key=b"user_001"), metadata=metadata)
    return resp.success and resp.value == new_value

def test_query_data(stub, metadata):
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
    ), metadata=metadata)
    return resp.success and len(resp.rows) >= 0

def test_delete_data(stub, metadata):
    resp = stub.Delete(rpc_pb2.DeleteRequest(schema=SCHEMA, table=TABLE_NAME, key=b"user_001"), metadata=metadata)
    return resp.success

def test_verify_delete(stub, metadata):
    resp = stub.Get(rpc_pb2.GetRequest(schema=SCHEMA, table=TABLE_NAME, key=b"user_001"), metadata=metadata)
    return resp.success and (resp.value is None or resp.value == b"")

def test_error_handling(stub, metadata):
    try:
        resp = stub.Get(rpc_pb2.GetRequest(schema=SCHEMA, table="nonexistent_table", key=b"test"), metadata=metadata)
        return not resp.success
    except grpc.RpcError as e:
        return True

# ==================== SQL 查询全链路测试 ====================

def test_create_sql_table(stub, metadata):
    """创建用于SQL查询测试的表"""
    resp = stub.CreateTable(rpc_pb2.CreateTableRequest(
        schema=SCHEMA,
        table_name=SQL_TABLE_NAME,
        columns=[
            rpc_pb2.ColumnDef(name="id", column_type=1),      # INT64
            rpc_pb2.ColumnDef(name="name", column_type=0),    # STRING
            rpc_pb2.ColumnDef(name="age", column_type=1),     # INT64
            rpc_pb2.ColumnDef(name="score", column_type=3),   # FLOAT
        ]
    ), metadata=metadata)
    if not resp.success:
        return False
    
    time.sleep(1)
    return True

def test_add_sql_data(stub, metadata):
    """添加测试数据到SQL测试表"""
    test_rows = [
        {"id": 1, "name": "Alice", "age": 25, "score": 95.5},
        {"id": 2, "name": "Bob", "age": 30, "score": 88.0},
        {"id": 3, "name": "Charlie", "age": 28, "score": 92.3},
        {"id": 4, "name": "David", "age": 22, "score": 85.0},
        {"id": 5, "name": "Eve", "age": 35, "score": 97.8},
    ]
    
    for row in test_rows:
        resp = stub.AddRow(rpc_pb2.AddRowRequest(
            schema=SCHEMA,
            table_name=SQL_TABLE_NAME,
            row=rpc_pb2.Row(
                row_type=0,
                version=1,
                data=[
                    encode_field(row["id"], 1),      # id: INT64
                    encode_field(row["name"], 0),    # name: STRING
                    encode_field(row["age"], 1),     # age: INT64
                    encode_field(row["score"], 3),   # score: FLOAT
                ]
            )
        ), metadata=metadata)
        if not resp.success:
            return False
    
    time.sleep(0.5)
    return True

def test_sql_query_select(stub, metadata):
    """测试SQL SELECT查询"""
    resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(
        schema=SCHEMA,
        sql="SELECT * FROM {}".format(SQL_TABLE_NAME)
    ), metadata=metadata)
    
    if not resp.success:
        print(f"    SQL查询失败: {resp.message}")
        return False
    
    print(f"    查询结果: {len(resp.rows)} 行")
    for i, row in enumerate(resp.rows):
        values = []
        for field in row.values:
            if field.HasField("string_value"):
                values.append(field.string_value)
            elif field.HasField("int64_value"):
                values.append(str(field.int64_value))
            elif field.HasField("float_value"):
                values.append(str(field.float_value))
        print(f"      行{i+1}: {values}")
    
    return len(resp.rows) >= 0

def test_sql_query_filter(stub, metadata):
    """测试带过滤条件的SQL查询"""
    resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(
        schema=SCHEMA,
        sql="SELECT name, age FROM {} WHERE age > 25".format(SQL_TABLE_NAME)
    ), metadata=metadata)
    
    if not resp.success:
        print(f"    SQL查询失败: {resp.message}")
        return False
    
    print(f"    过滤查询结果: {len(resp.rows)} 行")
    for i, row in enumerate(resp.rows):
        values = []
        for field in row.values:
            if field.HasField("string_value"):
                values.append(field.string_value)
            elif field.HasField("int64_value"):
                values.append(str(field.int64_value))
        print(f"      行{i+1}: {values}")
    
    return len(resp.rows) >= 1

def test_sql_query_aggregate(stub, metadata):
    """测试SQL聚合查询"""
    resp = stub.SqlQuery(rpc_pb2.SqlQueryRequest(
        schema=SCHEMA,
        sql="SELECT COUNT(*), AVG(age), MAX(score) FROM {}".format(SQL_TABLE_NAME)
    ), metadata=metadata)
    
    if not resp.success:
        print(f"    SQL聚合查询失败: {resp.message}")
        return False
    
    print(f"    聚合查询结果: {len(resp.rows)} 行")
    for row in resp.rows:
        values = []
        for field in row.values:
            if field.HasField("int64_value"):
                values.append(str(field.int64_value))
            elif field.HasField("float_value"):
                values.append(str(field.float_value))
        print(f"      聚合结果: {values}")
    
    return len(resp.rows) >= 1

def test_drop_sql_table(stub, metadata):
    """删除SQL测试表"""
    resp = stub.DropTable(rpc_pb2.DropTableRequest(
        schema=SCHEMA,
        table_name=SQL_TABLE_NAME
    ), metadata=metadata)
    return resp.success

if __name__ == "__main__":
    run_tests()
