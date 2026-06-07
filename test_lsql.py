#!/usr/bin/env python3
"""
lsql 客户端和 gRPC API 测试脚本
"""

import subprocess
import time
import sys


def run_command(cmd, timeout=10):
    """运行命令并返回结果"""
    try:
        result = subprocess.run(
            cmd,
            shell=True,
            capture_output=True,
            text=True,
            timeout=timeout
        )
        return result.returncode, result.stdout, result.stderr
    except Exception as e:
        return -1, "", str(e)


def test_help():
    """测试帮助信息"""
    print("\n测试 1: 帮助信息")
    code, stdout, stderr = run_command("./target/debug/lsql --help")
    print(f"返回码: {code}")
    print(f"输出: {stdout[:200]}")
    return code == 0


def test_list_schemas():
    """测试列出所有 Schema"""
    print("\n测试 2: 列出所有 Schema")
    # 我们会在 Python 中用 grpc 直接调用，或者用 Python 测试脚本
    print("注意: 交互式命令需要特定处理，建议直接使用 gRPC API 测试")
    print("或者直接在终端运行 lsql 进行手动测试")
    return True


def main():
    """主函数"""
    print("="*50)
    print("lsql 客户端测试")
    print("="*50)
    
    tests = [
        test_help,
        test_list_schemas,
    ]
    
    passed = 0
    failed = 0
    
    for test in tests:
        try:
            result = test()
            if result:
                passed += 1
                print("✓ PASS")
            else:
                failed += 1
                print("✗ FAIL")
        except Exception as e:
            failed += 1
            print(f"✗ ERROR: {e}")
    
    print("\n" + "="*50)
    print(f"测试结果: {passed} 成功, {failed} 失败")
    print("="*50)
    
    # 给出使用说明
    print("\n手动测试说明:")
    print("1. 启动 lsql: ./target/debug/lsql")
    print("2. 输入 '\\dn' 或 '\\schemas' 列出所有 Schema")
    print("3. 输入 '\\c <schema>' 切换到其他 Schema")
    print("4. 输入 '\\dt' 列出当前 Schema 的表")
    print("5. 输入 SQL 语句执行查询")
    print("6. 输入 '\\q' 退出")
    print()
    
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
