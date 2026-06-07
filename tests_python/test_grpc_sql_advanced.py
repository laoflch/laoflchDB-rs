#!/usr/bin/env python3
"""
gRPC SQL 高级查询测试 - 验证聚合、排序、分组等功能
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

TEST_DB = "./laoflch_db_grpc_advanced_test"
TEST_ADDR = "127.0.0.1:19777"
SERVER_BIN = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "release", "laoflchDB-rust")

def run_grpc_advanced_sql_test():
    os.chdir(os.path.dirname(os.path.abspath(__file__)))

    print("=" * 70)
    print("Python 自动回归测试: gRPC SQL 高级查询")
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
        text=True
    )
    time.sleep(4)
    print(f"    ✓ 服务已启动 PID={server_proc.pid} 监听 {TEST_ADDR}")

    try:
        print("\n[4/6] 等待服务就绪...")
        max_retries = 10
        for i in range(max_retries):
            try:
                channel = grpc.insecure_channel(TEST_ADDR)
                stub = rpc_pb2_grpc.LaoflchDbStub(channel)
                stub.ListTables(rpc_pb2.ListTablesRequest(schema="sys"))
                print(f"    ✓ 服务已就绪 (尝试 {i+1}/{max_retries})")
                break
            except grpc.RpcError as e:
                print(f"    服务尚未就绪 (尝试 {i+1}/{max_retries}): {e.code()}")
                time.sleep(1)
                continue
        
        print("\n[5/6] 创建测试表...")
        
        # 删除旧表
        for table_name in ["employees", "scores"]:
            try:
                drop_req = rpc_pb2.DropTableRequest(
                    schema="sys",
                    table_name=table_name
                )
                stub.DropTable(drop_req)
                print(f"    - 已删除旧表 {table_name}")
            except grpc.RpcError as e:
                if e.code() == grpc.StatusCode.NOT_FOUND:
                    print(f"    - 表 {table_name} 不存在，跳过删除")
        
        # 创建 employees 表（用于测试聚合和分组）
        create_employee_req = rpc_pb2.CreateTableRequest(
            schema="sys",
            table_name="employees",
            columns=[
                rpc_pb2.ColumnDef(name="id", column_type=1),          # INT64
                rpc_pb2.ColumnDef(name="name", column_type=0),        # STRING
                rpc_pb2.ColumnDef(name="department", column_type=0),  # STRING
                rpc_pb2.ColumnDef(name="salary", column_type=3),      # FLOAT
                rpc_pb2.ColumnDef(name="age", column_type=1),         # INT64
            ]
        )
        create_resp = stub.CreateTable(create_employee_req)
        assert create_resp.success == True, "创建 employees 表失败"
        print("    ✓ 创建 employees 表成功")
        
        # 创建 scores 表（用于测试排序）
        create_score_req = rpc_pb2.CreateTableRequest(
            schema="sys",
            table_name="scores",
            columns=[
                rpc_pb2.ColumnDef(name="id", column_type=1),          # INT64
                rpc_pb2.ColumnDef(name="student", column_type=0),     # STRING
                rpc_pb2.ColumnDef(name="score", column_type=3),       # FLOAT
                rpc_pb2.ColumnDef(name="subject", column_type=0),     # STRING
            ]
        )
        create_resp = stub.CreateTable(create_score_req)
        assert create_resp.success == True, "创建 scores 表失败"
        print("    ✓ 创建 scores 表成功")
        
        # 等待表注册到 SQL 引擎
        print("    等待表注册到 SQL 引擎...")
        time.sleep(2)
        
        print("\n[6/6] 测试 SQL 高级查询...")
        
        # 插入 employees 数据
        employees_data = [
            (1, "Alice", "IT", 9500.0, 30),
            (2, "Bob", "HR", 7500.0, 25),
            (3, "Charlie", "IT", 8500.0, 35),
            (4, "David", "HR", 7200.0, 28),
            (5, "Eve", "IT", 9200.0, 40),
            (6, "Frank", "Sales", 8000.0, 32),
            (7, "Grace", "Sales", 7800.0, 29),
        ]
        
        for id, name, dept, salary, age in employees_data:
            add_req = rpc_pb2.AddRowRequest(
                schema="sys",
                table_name="employees",
                row=rpc_pb2.Row(
                    row_type=0,
                    version=1,
                    data=[
                        encode_field(id, 1),      # id
                        encode_field(name, 0),    # name
                        encode_field(dept, 0),    # department
                        encode_field(salary, 3),  # salary
                        encode_field(age, 1),     # age
                    ]
                )
            )
            add_resp = stub.AddRow(add_req)
            assert add_resp.success == True, f"插入 employee {id} 失败"
            print(f"    ✓ 插入 employee {id}: {name}")
        
        # 插入 scores 数据
        scores_data = [
            (1, "Alice", 95.5, "Math"),
            (2, "Bob", 88.0, "Math"),
            (3, "Charlie", 92.5, "Math"),
            (4, "Alice", 90.0, "Science"),
            (5, "Bob", 85.5, "Science"),
            (6, "David", 78.0, "Math"),
            (7, "Eve", 92.0, "Science"),
        ]
        
        for id, student, score, subject in scores_data:
            add_req = rpc_pb2.AddRowRequest(
                schema="sys",
                table_name="scores",
                row=rpc_pb2.Row(
                    row_type=0,
                    version=1,
                    data=[
                        encode_field(id, 1),       # id
                        encode_field(student, 0), # student
                        encode_field(score, 3),   # score
                        encode_field(subject, 0), # subject
                    ]
                )
            )
            add_resp = stub.AddRow(add_req)
            assert add_resp.success == True, f"插入 score {id} 失败"
            print(f"    ✓ 插入 score {id}: {student} - {subject} = {score}")
        
        time.sleep(1)
        
        # ========== 测试聚合函数 ==========
        print("\n    === 测试聚合函数 ===")
        
        # 测试 COUNT(id) - 使用 COUNT(column) 而非 COUNT(*)
        print("\n    测试 COUNT(id):")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT COUNT(id) FROM employees"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, "COUNT(id) 查询失败"
        print(f"        ✓ COUNT(id) 查询成功，返回 {len(sql_resp.rows)} 行")
        
        # 测试 COUNT(department)
        print("\n    测试 COUNT(department):")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT COUNT(department) FROM employees"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, "COUNT(department) 查询失败"
        print(f"        ✓ COUNT(department) 查询成功")
        
        # 测试带 WHERE 的 COUNT
        print("\n    测试 COUNT with WHERE:")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT COUNT(id) FROM employees WHERE salary > 8000"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, "COUNT with WHERE 查询失败"
        print(f"        ✓ COUNT with WHERE 查询成功")
        
        # 测试 SUM
        print("\n    测试 SUM(salary):")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT SUM(salary) FROM employees"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, "SUM(salary) 查询失败"
        print(f"        ✓ SUM(salary) 查询成功")
        
        # 测试 AVG
        print("\n    测试 AVG(age):")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT AVG(age) FROM employees"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, "AVG(age) 查询失败"
        print(f"        ✓ AVG(age) 查询成功")
        
        # 测试 MIN/MAX
        print("\n    测试 MIN/MAX:")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT MIN(salary), MAX(salary) FROM employees"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, "MIN/MAX 查询失败"
        print(f"        ✓ MIN/MAX 查询成功")
        
        # 测试多个聚合函数（简化版）
        print("\n    测试多个聚合函数:")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT COUNT(id), SUM(salary) FROM employees"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, "多个聚合函数查询失败"
        print(f"        ✓ 多个聚合函数查询成功")
        
        # ========== 测试 GROUP BY ==========
        print("\n    === 测试 GROUP BY ===")
        
        # 测试按部门分组统计人数
        print("\n    测试 GROUP BY department:")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT department, COUNT(id) as cnt FROM employees GROUP BY department ORDER BY department"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, "GROUP BY 查询失败"
        print(f"        ✓ GROUP BY 查询成功，返回 {len(sql_resp.rows)} 行")
        for row in sql_resp.rows:
            dept = row.values[0].string_value
            cnt = row.values[1].int64_value
            print(f"            部门: {dept}, 人数: {cnt}")
        
        # 测试按部门分组统计平均工资
        print("\n    测试 GROUP BY + AVG:")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT department, AVG(salary) as avg_salary FROM employees GROUP BY department ORDER BY department"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, "GROUP BY + AVG 查询失败"
        print(f"        ✓ GROUP BY + AVG 查询成功，返回 {len(sql_resp.rows)} 行")
        
        # ========== 测试 ORDER BY ==========
        print("\n    === 测试 ORDER BY ===")
        
        # 测试 ORDER BY ASC
        print("\n    测试 ORDER BY salary ASC:")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT name, salary FROM employees ORDER BY salary ASC"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, "ORDER BY ASC 查询失败"
        print(f"        ✓ ORDER BY ASC 查询成功，返回 {len(sql_resp.rows)} 行")
        
        # 测试 ORDER BY DESC
        print("\n    测试 ORDER BY salary DESC:")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT name, salary FROM employees ORDER BY salary DESC"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, "ORDER BY DESC 查询失败"
        print(f"        ✓ ORDER BY DESC 查询成功，返回 {len(sql_resp.rows)} 行")
        
        # 测试 ORDER BY 多列
        print("\n    测试 ORDER BY department ASC, salary DESC:")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT department, name, salary FROM employees ORDER BY department ASC, salary DESC"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, "ORDER BY 多列查询失败"
        print(f"        ✓ ORDER BY 多列查询成功，返回 {len(sql_resp.rows)} 行")
        
        # ========== 测试 LIMIT ==========
        print("\n    === 测试 LIMIT ===")
        
        # 测试 LIMIT
        print("\n    测试 LIMIT 3:")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT name, salary FROM employees ORDER BY salary DESC LIMIT 3"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, "LIMIT 查询失败"
        assert len(sql_resp.rows) == 3, f"LIMIT 3 应返回 3 行，实际返回 {len(sql_resp.rows)} 行"
        print(f"        ✓ LIMIT 查询成功，返回 {len(sql_resp.rows)} 行")
        
        # 测试 LIMIT + OFFSET
        print("\n    测试 LIMIT 2 OFFSET 2:")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT name, salary FROM employees ORDER BY salary DESC LIMIT 2 OFFSET 2"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, "LIMIT OFFSET 查询失败"
        assert len(sql_resp.rows) == 2, f"LIMIT 2 OFFSET 2 应返回 2 行，实际返回 {len(sql_resp.rows)} 行"
        print(f"        ✓ LIMIT OFFSET 查询成功，返回 {len(sql_resp.rows)} 行")
        
        # ========== 测试复杂查询 ==========
        print("\n    === 测试复杂查询 ===")
        
        # 测试 GROUP BY + ORDER BY + LIMIT
        print("\n    测试 GROUP BY + ORDER BY + LIMIT:")
        sql_req = rpc_pb2.SqlQueryRequest(
            schema="sys",
            sql="SELECT department, COUNT(id) as cnt, AVG(salary) as avg_sal FROM employees GROUP BY department ORDER BY cnt DESC LIMIT 2"
        )
        sql_resp = stub.SqlQuery(sql_req)
        assert sql_resp.success == True, "复杂查询失败"
        print(f"        ✓ 复杂查询成功，返回 {len(sql_resp.rows)} 行")
        
        print("\n" + "=" * 70)
        print("SUCCESS! gRPC SQL 高级查询测试全部通过")
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
        print(f"数据保留在: {TEST_DB}")

if __name__ == "__main__":
    sys.exit(run_grpc_advanced_sql_test())
