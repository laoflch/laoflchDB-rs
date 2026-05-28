#!/usr/bin/env python3
import subprocess
import time
import sys
import os
import signal
import grpc

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import rpc_pb2
import rpc_pb2_grpc

TEST_DB = "./test_db_python_e2e"
TEST_ADDR = "127.0.0.1:19777"
SERVER_BIN = "../target/release/laoflchDB-rust"

def main():
    os.chdir(os.path.dirname(os.path.abspath(__file__)))

    print("=" * 60)
    print("Python 自动回归测试: gRPC 端到端数据写入和读取验证")
    print("=" * 60)

    print("\n[1/7] 编译 Rust release 版本...")
    result = subprocess.run(["cargo", "build", "--release"], cwd="..", capture_output=True)
    if result.returncode != 0:
        print("编译失败:", result.stderr.decode())
        return 1
    print("    ✓ 编译完成")

    print("\n[2/7] 清理旧测试数据...")
    subprocess.run(["rm", "-rf", TEST_DB], cwd="..")
    print("    ✓ 数据目录已清理")

    print("\n[3/7] 启动 laoflchDB gRPC 服务后台进程...")
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
    time.sleep(2.5)
    print(f"    ✓ 服务已启动 PID={server_proc.pid} 监听 {TEST_ADDR}")

    try:
        print("\n[4/7] 连接 gRPC 客户端...")
        channel = grpc.insecure_channel(TEST_ADDR)
        stub = rpc_pb2_grpc.LaoflchDbStub(channel)
        print("    ✓ gRPC channel 已连接")

        print("\n[5/7] 调用 ListTables: 获取当前数据库表...")
        list_resp = stub.ListTables(rpc_pb2.ListTablesRequest(), timeout=5)
        print(f"    表列表 = {list(list_resp.tables)}")
        assert 'user' in list(list_resp.tables), "初始化 user 表应该存在"
        print("    ✓ user 表自动初始化正确")

        print("\n[6/7] 通过 gRPC 写入数据 Put (user表)...")
        test_key = b"test_python_key_001"
        test_value = b"{\"name\": \"laoflch\", \"score\": 100}"
        stub.Put(rpc_pb2.PutRequest(table="user", key=test_key, value=test_value))
        print(f"    ✓ 写入 key={test_key.decode()} value_len={len(test_value)}")

        print("\n[7/7] 通过 gRPC 读取刚写入的数据 Get 并校验...")
        get_resp = stub.Get(rpc_pb2.GetRequest(table="user", key=test_key))
        print("    读取结果:")
        print(f"        found = {get_resp.found}")
        print(f"        value = {get_resp.value}")
        print(f"        value_utf8 = {get_resp.value.decode('utf-8')}")
        assert get_resp.found == True
        assert get_resp.value == test_value
        print("    ✓ ✓ ✓ 数据校验一致: 写入等于读出")

    except Exception as e:
        print(f"\n    ✗ 测试失败: {type(e).__name__}: {e}")
        return 1
    finally:
        print("\n--- 终止服务进程 ---")
        os.killpg(os.getpgid(server_proc.pid), signal.SIGTERM)
        try:
            server_proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            server_proc.kill()
        subprocess.run(["rm", "-rf", TEST_DB], cwd="..")

    print("\n" + "=" * 60)
    print("SUCCESS! Python 自动回归验证全部通过")
    print("=" * 60)
    return 0

if __name__ == "__main__":
    sys.exit(main())
