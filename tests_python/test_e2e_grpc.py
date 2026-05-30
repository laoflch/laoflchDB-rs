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

TEST_DB = "./laoflch_db_data"
TEST_ADDR = "127.0.0.1:19777"
SERVER_BIN = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "release", "laoflchDB-rust")

def main():
    os.chdir(os.path.dirname(os.path.abspath(__file__)))

    print("=" * 60)
    print("Python 自动回归测试: gRPC 端到端数据写入和读取验证")
    print("=" * 60)

    print("\n[1/5] 编译 Rust release 版本...")
    result = subprocess.run(["cargo", "build", "--release"], cwd="..", capture_output=True)
    if result.returncode != 0:
        print("编译失败:", result.stderr.decode())
        return 1
    print("    ✓ 编译完成")

    print("\n[2/5] 启动 laoflchDB gRPC 服务后台进程...")
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
        print("\n[3/5] 连接 gRPC 客户端...")
        channel = grpc.insecure_channel(TEST_ADDR)
        stub = rpc_pb2_grpc.LaoflchDbStub(channel)
        print("    ✓ gRPC channel 已连接")

        print("\n[4/5] 通过 gRPC 写入多笔测试数据到 user 表...")
        test_data = [
            (b"user_grpc_001", b'{"user_id": 1001, "password": "grpc_pass_001"}'),
            (b"user_grpc_002", b'{"user_id": 1002, "password": "grpc_pass_002"}'),
            (b"user_grpc_003", b'{"user_id": 1003, "password": "grpc_pass_003"}'),
        ]
        
        print("    准备插入的数据:")
        for key, value in test_data:
            print(f"        key={key.decode()}")
            print(f"        value={value.decode()}")
            stub.Put(rpc_pb2.PutRequest(table="user", key=key, value=value))
            print(f"        ✓ 写入成功")
            print()

        print("\n[5/5] 通过 gRPC 读取并校验所有写入的数据...")
        for key, expected_value in test_data:
            get_resp = stub.Get(rpc_pb2.GetRequest(table="user", key=key))
            print(f"    读取 key={key.decode()}:")
            print(f"        found = {get_resp.found}")
            print(f"        value = {get_resp.value.decode('utf-8')}")
            assert get_resp.found == True
            assert get_resp.value == expected_value
            print(f"        ✓ 数据校验通过")

        print("\n" + "=" * 60)
        print("SUCCESS! Python 自动回归验证全部通过")
        print("=" * 60)
        print(f"数据保留在: {TEST_DB}")

    except Exception as e:
        print(f"\n    ✗ 测试失败: {type(e).__name__}: {e}")
        import traceback
        traceback.print_exc()
        return 1
    finally:
        print("\n--- 终止服务进程 ---")
        os.killpg(os.getpgid(server_proc.pid), signal.SIGTERM)
        try:
            server_proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            server_proc.kill()
        # 不删除数据！
        print(f"数据保留在: {TEST_DB}")

    return 0

if __name__ == "__main__":
    sys.exit(main())
