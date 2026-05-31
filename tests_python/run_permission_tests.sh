#!/usr/bin/env python3
"""
权限配置综合测试运行器
运行所有权限相关测试：
1. REST API 权限测试
2. gRPC 权限测试
"""
import subprocess
import sys
import os

def run_test(script_name: str, description: str) -> bool:
    """运行单个测试脚本"""
    print("\n" + "="*70)
    print(f"运行: {description}")
    print("="*70)
    
    script_path = os.path.join(os.path.dirname(os.path.abspath(__file__)), script_name)
    
    if not os.path.exists(script_path):
        print(f"错误: 测试脚本不存在: {script_path}")
        return False
    
    try:
        result = subprocess.run(
            [sys.executable, script_path],
            cwd=os.path.dirname(os.path.abspath(__file__))
        )
        return result.returncode == 0
    except Exception as e:
        print(f"错误: {e}")
        return False


def main():
    print("="*70)
    print(" "*15 + "权限配置综合测试套件")
    print("="*70)
    
    tests = [
        ("test_permission.py", "REST API 权限配置测试"),
        ("test_permission_grpc.py", "gRPC 权限配置测试"),
    ]
    
    results = []
    for script, desc in tests:
        result = run_test(script, desc)
        results.append((desc, result))
    
    print("\n" + "="*70)
    print("最终测试结果汇总")
    print("="*70)
    
    passed = sum(1 for _, r in results if r)
    failed = len(results) - passed
    
    for desc, result in results:
        status = "✓ 通过" if result else "✗ 失败"
        print(f"  {desc}: {status}")
    
    print(f"\n总计: {passed} 通过, {failed} 失败")
    
    if failed > 0:
        print("\n注意: 部分测试失败，请检查上述输出。")
    else:
        print("\n✓ 所有权限测试通过！")
    
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
