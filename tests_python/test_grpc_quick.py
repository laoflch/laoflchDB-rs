#!/usr/bin/env python3
"""
gRPC SQL 查询快速测试 - 连接到现有的服务
"""
import grpc
import sys
import os

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import rpc_pb2
import rpc_pb2_grpc

TEST_ADDR = "127.0.0.1:19777"

def main():
    print("=" * 70)
    print("Python 自动回归测试: gRPC SQL 查询 (快速模式)")
    print(f"目标服务: {TEST_ADDR}")
    print("=" * 70)

    try:
        print("\n[1/2] 连接 gRPC 客户端...")
        channel = grpc.insecure_channel(TEST_ADDR)
        stub = rpc_pb2_grpc.LaoflchDbStub(channel)
        print("    ✓ gRPC channel 已连接")

        print("\n[2/2] 测试 SQL 查询...")

        # 测试全表查询
        print("\n    测试全表查询:")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT * FROM test_person"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, "SQL 查询失败"
        print(f"        ✓ 查询成功，返回 {len(sql_resp.rows)} 行")

        # 测试 OR 条件
        print("\n    测试 OR 条件 (age < 30 OR age > 40):")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT name, age FROM test_person WHERE age < 30 OR age > 40"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, "SQL 查询失败"
        assert len(sql_resp.rows) == 1, f"应返回 1 行，实际返回 {len(sql_resp.rows)} 行"
        assert sql_resp.rows[0].values[0].string_value == "Bob", "name 应为 Bob"
        assert sql_resp.rows[0].values[1].int64_value == 25, "age 应为 25"
        print(f"        ✓ 查询成功，返回 {len(sql_resp.rows)} 行")

        # 测试同一列多个 OR 条件
        print("\n    测试同一列多个 OR (age = 25 OR age = 30 OR age = 35):")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT name, age FROM test_person WHERE age = 25 OR age = 30 OR age = 35"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, "SQL 查询失败"
        assert len(sql_resp.rows) == 3, f"应返回 3 行，实际返回 {len(sql_resp.rows)} 行"
        print(f"        ✓ 查询成功，返回 {len(sql_resp.rows)} 行")

        # 测试 AND 条件
        print("\n    测试 AND 条件 (age > 25 AND score > 90):")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT name, age, score FROM test_person WHERE age > 25 AND score > 90"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, "SQL 查询失败"
        assert len(sql_resp.rows) == 2, f"应返回 2 行，实际返回 {len(sql_resp.rows)} 行"
        print(f"        ✓ 查询成功，返回 {len(sql_resp.rows)} 行")

        # 测试组合逻辑表达式
        print("\n    测试组合逻辑 ((age > 25 AND age < 40) OR score > 92):")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT name, age, score FROM test_person WHERE (age > 25 AND age < 40) OR score > 92"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, "SQL 查询失败"
        assert len(sql_resp.rows) == 2, f"应返回 2 行，实际返回 {len(sql_resp.rows)} 行"
        print(f"        ✓ 查询成功，返回 {len(sql_resp.rows)} 行")

        # 测试数据类型验证
        print("\n    测试数据类型验证:")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT id, name, age, score FROM test_person WHERE id = 1"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert len(sql_resp.rows) == 1, "应返回 1 行"
        
        row = sql_resp.rows[0]
        assert row.values[0].HasField('int64_value'), "id 应为 int64"
        assert row.values[1].HasField('string_value'), "name 应为 string"
        assert row.values[2].HasField('int64_value'), "age 应为 int64"
        assert row.values[3].HasField('float_value'), "score 应为 float"
        
        print(f"        ✓ id={row.values[0].int64_value} (int64)")
        print(f"        ✓ name='{row.values[1].string_value}' (string)")
        print(f"        ✓ age={row.values[2].int64_value} (int64)")
        print(f"        ✓ score={row.values[3].float_value} (float)")

        print("\n" + "=" * 70)
        print("SUCCESS! gRPC SQL 查询测试全部通过")
        print("=" * 70)

        return 0

    except Exception as e:
        print(f"\n    ✗ 测试失败: {type(e).__name__}: {e}")
        import traceback
        traceback.print_exc()
        return 1

if __name__ == "__main__":
    sys.exit(main())
