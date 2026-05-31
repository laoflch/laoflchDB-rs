#!/usr/bin/env python3
import subprocess
import time
import sys
import os
import signal

# Add the current directory to the Python path
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import grpc
import rpc_pb2
import rpc_pb2_grpc

TEST_DB = "./test_query_db"
TEST_ADDR = "127.0.0.1:19998"


def print_result(call_name, message, indent=2):
    print(f"\n>>> [gRPC 结果] {call_name}: ")
    for line in str(message).split('\n'):
        if line.strip():
            print(f"{' '*indent}{line}")


def main():
    # Get the project root directory
    test_dir = os.path.dirname(os.path.abspath(__file__))
    project_root = os.path.dirname(test_dir)
    os.chdir(test_dir)

    print("=" * 70)
    print("Python gRPC Query 接口自动测试")
    print("=" * 70)

    print("\n--- 清理并启动服务 ---")
    # Clean up test directory
    test_db_path = os.path.join(project_root, TEST_DB)
    subprocess.run(["rm", "-rf", test_db_path], cwd=project_root, check=True)

    # Build the binary first if not exists
    binary_path = os.path.join(project_root, "target/release/laoflchDB-rust")
    if not os.path.exists(binary_path):
        print("--- 正在构建项目 ---")
        subprocess.run(["cargo", "build", "--release"], cwd=project_root, check=True)

    # Start the server
    server_proc = subprocess.Popen(
        [binary_path, "start", "--addr", TEST_ADDR, "--db-path", TEST_DB],
        cwd=project_root,
        preexec_fn=os.setsid
    )
    # Wait a bit for server to start
    print(f"服务 PID {server_proc.pid}")
    time.sleep(5)

    try:
        channel = grpc.insecure_channel(TEST_ADDR)
        stub = rpc_pb2_grpc.LaoflchDbStub(channel)

        print("\n--- 1. 创建表 ---")
        create_table_resp = stub.CreateTable(rpc_pb2.CreateTableRequest(
            schema="sys",
            table_name="users",
            columns=[
                rpc_pb2.ColumnDef(name="id", column_type=1),  # INTEGER
                rpc_pb2.ColumnDef(name="name", column_type=0),  # STRING
                rpc_pb2.ColumnDef(name="age", column_type=1)  # INTEGER
            ]
        ))
        print_result("创建表", create_table_resp)
        assert create_table_resp.success

        print("\n--- 2. 插入数据 ---")
        # User 1
        add_row_resp1 = stub.AddRow(rpc_pb2.AddRowRequest(
            schema="sys",
            table_name="users",
            row=rpc_pb2.Row(
                row_type=rpc_pb2.ROW_TYPE_NORMAL,
                version=1,
                data=[
                    (1).to_bytes(8, 'big'),
                    b"Alice",
                    (30).to_bytes(8, 'big')
                ]
            )
        ))
        print_result("插入用户 1", add_row_resp1)
        assert add_row_resp1.success
        user1_id = add_row_resp1.row_id

        # User 2
        add_row_resp2 = stub.AddRow(rpc_pb2.AddRowRequest(
            schema="sys",
            table_name="users",
            row=rpc_pb2.Row(
                row_type=rpc_pb2.ROW_TYPE_NORMAL,
                version=1,
                data=[
                    (2).to_bytes(8, 'big'),
                    b"Bob",
                    (25).to_bytes(8, 'big')
                ]
            )
        ))
        print_result("插入用户 2", add_row_resp2)
        assert add_row_resp2.success

        # User 3
        add_row_resp3 = stub.AddRow(rpc_pb2.AddRowRequest(
            schema="sys",
            table_name="users",
            row=rpc_pb2.Row(
                row_type=rpc_pb2.ROW_TYPE_NORMAL,
                version=1,
                data=[
                    (3).to_bytes(8, 'big'),
                    b"Charlie",
                    (35).to_bytes(8, 'big')
                ]
            )
        ))
        print_result("插入用户 3", add_row_resp3)
        assert add_row_resp3.success

        print("\n--- 3. 测试简单查询 ---")
        # Query all (no filters)
        query_resp = stub.Query(rpc_pb2.QueryRequest(
            schema="sys",
            table_filters=[
                rpc_pb2.TableFilter(
                    table_name="users",
                    column_filters=[]
                )
            ]
        ))
        print_result("查询所有用户", query_resp)
        assert query_resp.success
        assert len(query_resp.rows) == 3

        print("\n--- 4. 测试等值查询 ---")
        query_resp_eq = stub.Query(rpc_pb2.QueryRequest(
            schema="sys",
            table_filters=[
                rpc_pb2.TableFilter(
                    table_name="users",
                    column_filters=[
                        rpc_pb2.ColumnFilter(
                            column_name="name",
                            conditions=[
                                rpc_pb2.ColumnFilterCondition(
                                    op=rpc_pb2.FILTER_OPERATOR_EQ,
                                    value=rpc_pb2.Field(
                                        string_value=rpc_pb2.StringValue(value="Alice")
                                    )
                                )
                            ]
                        )
                    ]
                )
            ]
        ))
        print_result("查询名字为 Alice 的用户", query_resp_eq)
        assert query_resp_eq.success
        assert len(query_resp_eq.rows) == 1

        print("\n--- 5. 测试大于查询 ---")
        query_resp_gt = stub.Query(rpc_pb2.QueryRequest(
            schema="sys",
            table_filters=[
                rpc_pb2.TableFilter(
                    table_name="users",
                    column_filters=[
                        rpc_pb2.ColumnFilter(
                            column_name="age",
                            conditions=[
                                rpc_pb2.ColumnFilterCondition(
                                    op=rpc_pb2.FILTER_OPERATOR_GT,
                                    value=rpc_pb2.Field(
                                        integer_value=rpc_pb2.IntegerValue(value=28)
                                    )
                                )
                            ]
                        )
                    ]
                )
            ]
        ))
        print_result("查询年龄大于 28 的用户", query_resp_gt)
        assert query_resp_gt.success
        assert len(query_resp_gt.rows) == 2

        print("\n--- 6. 测试 limit 和 offset ---")
        query_resp_limit = stub.Query(rpc_pb2.QueryRequest(
            schema="sys",
            table_filters=[
                rpc_pb2.TableFilter(
                    table_name="users",
                    column_filters=[]
                )
            ],
            limit=2,
            offset=1
        ))
        print_result("测试 limit 2 offset 1", query_resp_limit)
        assert query_resp_limit.success

        print("\n" + "=" * 70)
        print("✅ SUCCESS: 所有 Query 接口测试通过")
        print("=" * 70)

    except Exception as e:
        print(f"\n❌ ERROR: {e}")
        import traceback
        traceback.print_exc()
        raise
    finally:
        # Clean up
        os.killpg(os.getpgid(server_proc.pid), signal.SIGTERM)
        server_proc.wait()
        test_db_path = os.path.join(project_root, TEST_DB)
        subprocess.run(["rm", "-rf", test_db_path], cwd=project_root, check=True)


if __name__ == "__main__":
    main()
