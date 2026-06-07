#!/usr/bin/env python3
"""
测试 ListSchemas gRPC API 功能
"""

import grpc
import sys
import os

# 确保可以导入生成的 protobuf 文件
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import rpc_pb2
import rpc_pb2_grpc


def test_list_schemas():
    """测试列出所有 schema"""
    print("测试 ListSchemas API...")
    
    with grpc.insecure_channel('127.0.0.1:19777') as channel:
        stub = rpc_pb2_grpc.LaoflchDbStub(channel)
        
        try:
            request = rpc_pb2.ListSchemasRequest()
            response = stub.ListSchemas(request)
            print(f"✓ ListSchemas 调用成功!")
            print(f"✓ Success: {response.success}")
            print(f"✓ Schemas: {response.schemas}")
            return True
        except Exception as e:
            print(f"✗ ListSchemas 调用失败: {e}")
            return False


def main():
    """主函数"""
    print("="*50)
    print("测试 ListSchemas API")
    print("="*50)

    try:
        test_list_schemas()
    except Exception as e:
        print(f"\n错误: {e}")
        print("确保 laoflchDB 服务是否在运行?")
        sys.exit(1)


if __name__ == "__main__":
    main()

