#!/usr/bin/env python3
"""
gRPC SQL 查询测试 - 验证数据类型正确性和查询功能
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

TEST_DB = "./laoflch_db_grpc_test"
TEST_ADDR = "127.0.0.1:19777"
SERVER_BIN = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "release", "laoflchDB-rust")

def run_grpc_test():
    os.chdir(os.path.dirname(os.path.abspath(__file__)))

    print("=" * 70)
    print("Python 自动回归测试: gRPC SQL 查询数据类型验证")
    print("=" * 70)

    print("\n[1/6] 编译 Rust release 版本...")
    result = subprocess.run(["cargo", "build", "--release"], cwd="..", capture_output=True)
    if result.returncode != 0:
        print("编译失败:", result.stderr.decode())
        return 1
    print("    ✓ 编译完成")

    print("\n[2/6] 清理旧测试数据...")
    subprocess.run(["rm", "-rf", TEST_DB], capture_output=True)
    print("    ✓ 清理完成")

    print("\n[3/6] 启动 laoflchDB gRPC 服务后台进程...")
    cmd = [
        SERVER_BIN,
        "start",
        "--addr", TEST_ADDR,
        "--db-path", TEST_DB
    ]
    server_proc = subprocess.Popen(
        cmd,
        cwd="..",
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        preexec_fn=os.setsid
    )
    time.sleep(4)
    print(f"    ✓ 服务已启动 PID={server_proc.pid} 监听 {TEST_ADDR}")

    try:
        print("\n[4/6] 连接 gRPC 客户端...")
        channel = grpc.insecure_channel(TEST_ADDR)
        stub = rpc_pb2_grpc.LaoflchDbStub(channel)
        print("    ✓ gRPC channel 已连接")

        print("\n[5/6] 创建测试表...")
        try:
            drop_req = rpc_pb2.DropTableRequest(
                schema="sys",
                table_name="test_grpc_sql"
            )
            stub.DropTable(drop_req)
            print("    - 已删除旧表")
        except:
            pass
        
        create_req = rpc_pb2.CreateTableRequest(
            schema="sys",
            table_name="test_grpc_sql",
            columns=[
                rpc_pb2.ColumnDef(name="id", column_type=1),      # INT64
                rpc_pb2.ColumnDef(name="name", column_type=4),    # STRING
                rpc_pb2.ColumnDef(name="age", column_type=1),     # INT64
                rpc_pb2.ColumnDef(name="score", column_type=3),   # FLOAT
            ]
        )
        create_resp = stub.CreateTable(create_req)
        assert create_resp.success == True, "创建表失败"
        print("    ✓ 创建表成功")

        print("\n[6/6] 插入测试数据...")
        # 使用 AddRow 插入数据
        row_data = [
            (1, "Alice", 30, 95.5),
            (2, "Bob", 25, 88.0),
            (3, "Charlie", 35, 92.5),
        ]

        for row_id, name, age, score in row_data:
            add_req = rpc_pb2.AddRowRequest(
                schema="sys",
                table_name="test_grpc_sql",
                row=rpc_pb2.Row(
                    row_type=0,
                    version=1,
                    data=[
                        str(row_id).encode('utf-8'),
                        name.encode('utf-8'),
                        str(age).encode('utf-8'),
                        str(score).encode('utf-8'),
                    ]
                )
            )
            add_resp = stub.AddRow(add_req)
            assert add_resp.success == True, f"插入行 {row_id} 失败"
            print(f"    ✓ 插入行 {row_id} 成功")

        print("\n[7/6] 测试 SQL 查询...")
        
        # 测试全表查询
        print("\n    测试全表查询:")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT * FROM test_grpc_sql"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, "SQL 查询失败"
        print(f"        ✓ 查询成功，返回 {len(sql_resp.rows)} 行")
        print(f"        ✓ 列名: {sql_resp.columns}")
        
        for row in sql_resp.rows:
            values = []
            for val in row.values:
                if val.HasField('int64_value'):
                    values.append(f"{val.int64_value} (int64)")
                elif val.HasField('float_value'):
                    values.append(f"{val.float_value} (float)")
                elif val.HasField('string_value'):
                    values.append(f"'{val.string_value}' (string)")
                elif val.HasField('bytes_value'):
                    values.append(f"bytes[{len(val.bytes_value)}]")
            print(f"        行数据: {values}")

        # 测试谓词下推查询
        print("\n    测试谓词下推 (age > 30):")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT name, age FROM test_grpc_sql WHERE age > 30"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, "SQL 查询失败"
        assert len(sql_resp.rows) == 2, f"应返回 2 行，实际返回 {len(sql_resp.rows)} 行"
        print(f"        ✓ 查询成功，返回 {len(sql_resp.rows)} 行")

        # 测试数据类型验证
        print("\n    测试数据类型验证:")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT id, name, age, score FROM test_grpc_sql WHERE id = 1"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert len(sql_resp.rows) == 1, "应返回 1 行"
        
        row = sql_resp.rows[0]
        assert row.values[0].HasField('int64_value'), "id 应为 int64"
        assert row.values[1].HasField('string_value'), "name 应为 string"
        assert row.values[2].HasField('int64_value'), "age 应为 int64"
        assert row.values[3].HasField('float_value'), "score 应为 float"
        
        assert row.values[0].int64_value == 1, f"id 应为 1，实际为 {row.values[0].int64_value}"
        assert row.values[1].string_value == "Alice", f"name 应为 Alice，实际为 {row.values[1].string_value}"
        assert row.values[2].int64_value == 30, f"age 应为 30，实际为 {row.values[2].int64_value}"
        assert abs(row.values[3].float_value - 95.5) < 0.01, f"score 应为 95.5，实际为 {row.values[3].float_value}"
        
        print(f"        ✓ id={row.values[0].int64_value} (int64)")
        print(f"        ✓ name='{row.values[1].string_value}' (string)")
        print(f"        ✓ age={row.values[2].int64_value} (int64)")
        print(f"        ✓ score={row.values[3].float_value} (float)")

        # 测试 OR 条件
        print("\n    测试 OR 条件 (age < 30 OR age > 40):")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT name, age FROM test_grpc_sql WHERE age < 30 OR age > 40"
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
            sql="SELECT name, age FROM test_grpc_sql WHERE age = 25 OR age = 30 OR age = 35"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, "SQL 查询失败"
        assert len(sql_resp.rows) == 3, f"应返回 3 行，实际返回 {len(sql_resp.rows)} 行"
        print(f"        ✓ 查询成功，返回 {len(sql_resp.rows)} 行")

        # 测试 AND 条件
        print("\n    测试 AND 条件 (age > 25 AND score > 90):")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT name, age, score FROM test_grpc_sql WHERE age > 25 AND score > 90"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, "SQL 查询失败"
        assert len(sql_resp.rows) == 2, f"应返回 2 行，实际返回 {len(sql_resp.rows)} 行"
        print(f"        ✓ 查询成功，返回 {len(sql_resp.rows)} 行")

        # 测试组合逻辑表达式
        print("\n    测试组合逻辑 ((age > 25 AND age < 40) OR score > 92):")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT name, age, score FROM test_grpc_sql WHERE (age > 25 AND age < 40) OR score > 92"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, "SQL 查询失败"
        assert len(sql_resp.rows) == 2, f"应返回 2 行，实际返回 {len(sql_resp.rows)} 行"
        print(f"        ✓ 查询成功，返回 {len(sql_resp.rows)} 行")

        # 测试 Limit 下推
        print("\n    测试 Limit 下推:")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT * FROM test_grpc_sql LIMIT 2"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert len(sql_resp.rows) == 2, f"应返回 2 行，实际返回 {len(sql_resp.rows)} 行"
        print(f"        ✓ 查询成功，返回 {len(sql_resp.rows)} 行")

        print("\n" + "=" * 70)
        print("SUCCESS! gRPC SQL 查询测试全部通过")
        print("=" * 70)
        print(f"数据保留在: {TEST_DB}")

        return 0

    except Exception as e:
        print(f"\n    ✗ 测试失败: {type(e).__name__}: {e}")
        import traceback
        traceback.print_exc()
        return 1
    finally:
        print("\n--- 终止服务进程 ---")
        try:
            os.killpg(os.getpgid(server_proc.pid), signal.SIGTERM)
            server_proc.wait(timeout=3)
        except:
            try:
                server_proc.kill()
            except:
                pass
        # 不删除数据
        print(f"数据保留在: {TEST_DB}")

if __name__ == "__main__":
    sys.exit(run_grpc_test())