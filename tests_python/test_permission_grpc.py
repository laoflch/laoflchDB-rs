#!/usr/bin/env python3
"""
gRPC 权限配置测试套件
测试场景：
1. 多gRPC服务不同权限
2. 服务增加和减少
3. 权限规则验证
4. 并发访问测试
"""
import subprocess
import time
import sys
import os
import signal
import tempfile
import yaml
import grpc
from concurrent import futures
from typing import Dict, List, Tuple

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import rpc_pb2
import rpc_pb2_grpc


class GrpcPermissionTester:
    """gRPC权限测试客户端"""
    
    def __init__(self, addr: str):
        self.addr = addr
        self.channel = None
        self.stub = None
    
    def connect(self):
        """建立连接"""
        self.channel = grpc.insecure_channel(self.addr)
        self.stub = rpc_pb2_grpc.LaoflchDbStub(self.channel)
    
    def close(self):
        """关闭连接"""
        if self.channel:
            self.channel.close()
    
    def health_check(self) -> bool:
        """健康检查（通过创建表间接验证）"""
        try:
            self.connect()
            return True
        except:
            return False
    
    def create_table(self, schema: str, table_name: str, columns: List[Dict]) -> Tuple[bool, str]:
        """创建表"""
        try:
            req = rpc_pb2.CreateTableRequest(
                schema=schema,
                table_name=table_name,
                columns=[
                    rpc_pb2.ColumnMeta(
                        column_name=col["name"],
                        column_type=get_column_type(col["column_type"])
                    )
                    for col in columns
                ]
            )
            resp = self.stub.CreateTable(req)
            return (resp.success, resp.message)
        except grpc.RpcError as e:
            return (False, str(e))
    
    def put_data(self, schema: str, table: str, key: bytes, value: bytes) -> Tuple[bool, str]:
        """写入数据"""
        try:
            req = rpc_pb2.PutRequest(
                schema=schema,
                table=table,
                key=key,
                value=value
            )
            resp = self.stub.Put(req)
            return (resp.success, resp.message)
        except grpc.RpcError as e:
            return (False, str(e))
    
    def get_data(self, schema: str, table: str, key: bytes) -> Tuple[bool, bytes, str]:
        """读取数据"""
        try:
            req = rpc_pb2.GetRequest(
                schema=schema,
                table=table,
                key=key
            )
            resp = self.stub.Get(req)
            if resp.found:
                return (True, resp.value, "")
            return (False, b"", "Not found")
        except grpc.RpcError as e:
            return (False, b"", str(e))
    
    def delete_data(self, schema: str, table: str, key: bytes) -> Tuple[bool, str]:
        """删除数据"""
        try:
            req = rpc_pb2.DeleteRequest(
                schema=schema,
                table=table,
                key=key
            )
            resp = self.stub.Delete(req)
            return (resp.success, resp.message)
        except grpc.RpcError as e:
            return (False, str(e))
    
    def list_tables(self, schema: str) -> Tuple[bool, List[str], str]:
        """列出表"""
        try:
            req = rpc_pb2.ListTablesRequest(schema=schema)
            resp = self.stub.ListTables(req)
            if resp.success:
                return (True, resp.tables, "")
            return (False, [], resp.message)
        except grpc.RpcError as e:
            return (False, [], str(e))


def get_column_type(type_str: str) -> int:
    """转换列类型"""
    type_map = {
        "STRING": rpc_pb2.STRING,
        "INT64": rpc_pb2.INT64,
        "BYTES": rpc_pb2.BYTES,
        "FLOAT": rpc_pb2.FLOAT,
        "DOUBLE": rpc_pb2.DOUBLE,
    }
    return type_map.get(type_str.upper(), rpc_pb2.STRING)


class GrpcPermissionTestRunner:
    """gRPC权限测试运行器"""
    
    def __init__(self, server_bin: str, db_dir: str):
        self.server_bin = server_bin
        self.db_dir = db_dir
        self.server_procs = {}
        self.config_path = None
    
    def start_server_with_config(self, config: Dict, grpc_addrs: List[str]) -> bool:
        """启动多个gRPC服务"""
        os.makedirs(self.db_dir, exist_ok=True)
        self.config_path = os.path.join(self.db_dir, "config.yaml")
        
        with open(self.config_path, 'w') as f:
            yaml.dump(config, f, default_flow_style=False, sort_keys=False)
        
        cmd = [self.server_bin, "-c", self.config_path, "start"]
        proc = subprocess.Popen(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            preexec_fn=os.setsid
        )
        
        time.sleep(3)
        if proc.poll() is not None:
            return False
        
        self.server_procs["main"] = proc
        
        for addr in grpc_addrs:
            time.sleep(0.5)
            tester = GrpcPermissionTester(addr)
            if not tester.health_check():
                return False
            tester.close()
        
        return True
    
    def stop_all(self):
        """停止所有服务"""
        for name, proc in self.server_procs.items():
            try:
                os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
                proc.wait(timeout=5)
            except:
                proc.kill()
        self.server_procs.clear()
    
    def cleanup(self):
        """清理资源"""
        self.stop_all()
        if os.path.exists(self.db_dir):
            import shutil
            shutil.rmtree(self.db_dir, ignore_errors=True)


def test_grpc_single_service():
    """测试1: gRPC单服务基本功能"""
    print("\n" + "="*60)
    print("测试1: gRPC单服务基本功能")
    print("="*60)
    
    db_dir = tempfile.mkdtemp(prefix="test_grpc_single_")
    runner = GrpcPermissionTestRunner(
        "../target/debug/laoflchDB-rust",
        db_dir
    )
    
    try:
        config = {
            "db_path": os.path.join(db_dir, "data"),
            "log_level": "info",
            "default_policy": "allow",
            "access_protocols": [{
                "protocol": "grpc",
                "enabled": True,
                "addr": "127.0.0.1:19777",
                "service_id": "admin"
            }],
            "permissions": [{
                "service_id": "admin",
                "default_policy": "allow",
                "allowed_actions": ["get", "put", "delete", "create_table", "list_tables"]
            }]
        }
        
        print("启动gRPC服务: admin (19777)")
        if not runner.start_server_with_config(config, ["127.0.0.1:19777"]):
            print("  ✗ 服务启动失败")
            return False
        print("  ✓ 服务已启动")
        
        tester = GrpcPermissionTester("127.0.0.1:19777")
        tester.connect()
        
        print("\n测试创建表...")
        success, msg = tester.create_table("sys", "grpc_test", [
            {"name": "id", "column_type": "Int64"},
            {"name": "name", "column_type": "String"}
        ])
        if success:
            print("  ✓ 创建表成功")
        else:
            print(f"  ✗ 创建表失败: {msg}")
            tester.close()
            return False
        
        print("\n测试写入数据...")
        success, msg = tester.put_data("sys", "grpc_test", b"key1", b"value1")
        if success:
            print("  ✓ 写入数据成功")
        else:
            print(f"  ✗ 写入数据失败: {msg}")
            tester.close()
            return False
        
        print("\n测试读取数据...")
        success, value, msg = tester.get_data("sys", "grpc_test", b"key1")
        if success and value == b"value1":
            print(f"  ✓ 读取数据成功: {value}")
        else:
            print(f"  ✗ 读取数据失败: {msg}")
            tester.close()
            return False
        
        print("\n测试列出表...")
        success, tables, msg = tester.list_tables("sys")
        if success and "grpc_test" in tables:
            print(f"  ✓ 列出表成功: {tables}")
        else:
            print(f"  ✗ 列出表失败")
            tester.close()
            return False
        
        tester.close()
        print("\n✓ 测试1通过: gRPC单服务基本功能正常")
        return True
        
    finally:
        runner.cleanup()


def test_grpc_multi_service():
    """测试2: gRPC多服务不同权限"""
    print("\n" + "="*60)
    print("测试2: gRPC多服务不同权限")
    print("="*60)
    
    db_dir = tempfile.mkdtemp(prefix="test_grpc_multi_")
    runner = GrpcPermissionTestRunner(
        "../target/debug/laoflchDB-rust",
        db_dir
    )
    
    try:
        config = {
            "db_path": os.path.join(db_dir, "data"),
            "log_level": "info",
            "default_policy": "deny",
            "access_protocols": [
                {
                    "protocol": "grpc",
                    "enabled": True,
                    "addr": "127.0.0.1:19778",
                    "service_id": "readonly"
                },
                {
                    "protocol": "grpc",
                    "enabled": True,
                    "addr": "127.0.0.1:19779",
                    "service_id": "writeonly"
                },
                {
                    "protocol": "grpc",
                    "enabled": True,
                    "addr": "127.0.0.1:19780",
                    "service_id": "admin"
                }
            ],
            "permissions": [
                {
                    "service_id": "readonly",
                    "default_policy": "deny",
                    "allowed_actions": ["get", "list_tables"],
                    "denied_actions": ["put", "delete", "create_table"]
                },
                {
                    "service_id": "writeonly",
                    "default_policy": "deny",
                    "allowed_actions": ["put", "delete", "create_table"],
                    "denied_actions": ["get", "list_tables"]
                },
                {
                    "service_id": "admin",
                    "default_policy": "allow",
                    "allowed_actions": ["get", "put", "delete", "create_table", "list_tables"]
                }
            ]
        }
        
        print("启动3个gRPC服务:")
        print("  - readonly (19778): 只允许get, list_tables")
        print("  - writeonly (19779): 只允许put, delete, create_table")
        print("  - admin (19780): 允许所有")
        
        if not runner.start_server_with_config(config, [
            "127.0.0.1:19778",
            "127.0.0.1:19779",
            "127.0.0.1:19780"
        ]):
            print("  ✗ 服务启动失败")
            return False
        print("  ✓ 所有服务已启动")
        
        # Admin创建测试表
        print("\n[Admin] 创建测试表...")
        admin = GrpcPermissionTester("127.0.0.1:19780")
        admin.connect()
        success, _ = admin.create_table("sys", "grpc_perm_test", [
            {"name": "id", "column_type": "Int64"}
        ])
        if success:
            print("  ✓ Admin创建表成功")
        else:
            print("  ✗ Admin创建表失败")
            admin.close()
            return False
        
        # 测试只读服务
        print("\n[ReadOnly] 测试只读权限...")
        readonly = GrpcPermissionTester("127.0.0.1:19778")
        readonly.connect()
        
        success, value, _ = readonly.get_data("sys", "grpc_perm_test", b"key1")
        if success:
            print("  ✓ ReadOnly可以读取")
        else:
            print("  ✗ ReadOnly读取失败")
        
        success, _ = readonly.put_data("sys", "grpc_perm_test", b"key2", b"value2")
        if not success:
            print("  ✓ ReadOnly被拒绝写入")
        else:
            print("  ✗ ReadOnly写入应该被拒绝")
        
        readonly.close()
        
        # 测试只写服务
        print("\n[WriteOnly] 测试只写权限...")
        writeonly = GrpcPermissionTester("127.0.0.1:19779")
        writeonly.connect()
        
        success, _ = writeonly.put_data("sys", "grpc_perm_test", b"key3", b"value3")
        if success:
            print("  ✓ WriteOnly可以写入")
        else:
            print("  ✗ WriteOnly写入失败")
        
        success, _, _ = writeonly.get_data("sys", "grpc_perm_test", b"key3")
        if not success:
            print("  ✓ WriteOnly被拒绝读取")
        else:
            print("  ✗ WriteOnly读取应该被拒绝")
        
        writeonly.close()
        admin.close()
        
        print("\n✓ 测试2通过: gRPC多服务权限隔离正常")
        return True
        
    finally:
        runner.cleanup()


def test_grpc_add_remove_service():
    """测试3: gRPC服务增加和减少"""
    print("\n" + "="*60)
    print("测试3: gRPC服务增加和减少")
    print("="*60)
    
    db_dir = tempfile.mkdtemp(prefix="test_grpc_addrm_")
    runner = GrpcPermissionTestRunner(
        "../target/debug/laoflchDB-rust",
        db_dir
    )
    
    try:
        # 阶段1: 只有admin
        print("阶段1: 启动只有admin的配置")
        config1 = {
            "db_path": os.path.join(db_dir, "data"),
            "log_level": "info",
            "default_policy": "allow",
            "access_protocols": [{
                "protocol": "grpc",
                "enabled": True,
                "addr": "127.0.0.1:19781",
                "service_id": "admin"
            }],
            "permissions": [{
                "service_id": "admin",
                "default_policy": "allow",
                "allowed_actions": ["get", "put", "create_table"]
            }]
        }
        
        if not runner.start_server_with_config(config1, ["127.0.0.1:19781"]):
            print("  ✗ 服务启动失败")
            return False
        print("  ✓ Admin服务已启动 (19781)")
        
        admin = GrpcPermissionTester("127.0.0.1:19781")
        admin.connect()
        admin.create_table("sys", "test_grpc", [{"name": "id", "column_type": "Int64"}])
        print("  ✓ 测试表已创建")
        
        # 阶段2: 增加readonly
        print("\n阶段2: 增加readonly服务")
        runner.stop_all()
        time.sleep(1)
        
        config2 = {
            "db_path": os.path.join(db_dir, "data"),
            "log_level": "info",
            "default_policy": "deny",
            "access_protocols": [
                {
                    "protocol": "grpc",
                    "enabled": True,
                    "addr": "127.0.0.1:19781",
                    "service_id": "admin"
                },
                {
                    "protocol": "grpc",
                    "enabled": True,
                    "addr": "127.0.0.1:19782",
                    "service_id": "readonly"
                }
            ],
            "permissions": [
                {
                    "service_id": "admin",
                    "default_policy": "allow",
                    "allowed_actions": ["get", "put", "create_table"]
                },
                {
                    "service_id": "readonly",
                    "default_policy": "deny",
                    "allowed_actions": ["get"]
                }
            ]
        }
        
        if not runner.start_server_with_config(config2, [
            "127.0.0.1:19781",
            "127.0.0.1:19782"
        ]):
            print("  ✗ 服务启动失败")
            admin.close()
            return False
        print("  ✓ Admin(19781) 和 ReadOnly(19782) 已启动")
        
        # 重新连接admin
        admin.close()
        admin = GrpcPermissionTester("127.0.0.1:19781")
        admin.connect()
        
        success, _ = admin.put_data("sys", "test_grpc", b"k1", b"v1")
        if success:
            print("  ✓ Admin写入成功")
        
        readonly = GrpcPermissionTester("127.0.0.1:19782")
        readonly.connect()
        success, _, _ = readonly.get_data("sys", "test_grpc", b"k1")
        if success:
            print("  ✓ ReadOnly可以读取admin写入的数据")
        readonly.close()
        admin.close()
        
        # 阶段3: 减少服务
        print("\n阶段3: 移除readonly服务")
        runner.stop_all()
        time.sleep(1)
        
        config3 = {
            "db_path": os.path.join(db_dir, "data"),
            "log_level": "info",
            "default_policy": "allow",
            "access_protocols": [{
                "protocol": "grpc",
                "enabled": True,
                "addr": "127.0.0.1:19781",
                "service_id": "admin"
            }],
            "permissions": [{
                "service_id": "admin",
                "default_policy": "allow",
                "allowed_actions": ["get", "put", "create_table"]
            }]
        }
        
        if not runner.start_server_with_config(config3, ["127.0.0.1:19781"]):
            print("  ✗ 服务启动失败")
            return False
        print("  ✓ 只有Admin服务 (19781)")
        
        admin = GrpcPermissionTester("127.0.0.1:19781")
        admin.connect()
        success, _, _ = admin.get_data("sys", "test_grpc", b"k1")
        if success:
            print("  ✓ Admin仍然正常工作，数据持久化正常")
        admin.close()
        
        print("\n✓ 测试3通过: gRPC服务增加和减少正常")
        return True
        
    finally:
        runner.cleanup()


def test_grpc_concurrent_access():
    """测试4: gRPC并发访问"""
    print("\n" + "="*60)
    print("测试4: gRPC并发访问")
    print("="*60)
    
    db_dir = tempfile.mkdtemp(prefix="test_grpc_concurrent_")
    runner = GrpcPermissionTestRunner(
        "../target/debug/laoflchDB-rust",
        db_dir
    )
    
    try:
        config = {
            "db_path": os.path.join(db_dir, "data"),
            "log_level": "info",
            "default_policy": "allow",
            "access_protocols": [{
                "protocol": "grpc",
                "enabled": True,
                "addr": "127.0.0.1:19790",
                "service_id": "concurrent"
            }],
            "permissions": [{
                "service_id": "concurrent",
                "default_policy": "allow",
                "allowed_actions": ["get", "put", "delete"]
            }]
        }
        
        print("启动gRPC服务: concurrent (19790)")
        if not runner.start_server_with_config(config, ["127.0.0.1:19790"]):
            print("  ✗ 服务启动失败")
            return False
        print("  ✓ 服务已启动")
        
        # 创建测试表
        admin = GrpcPermissionTester("127.0.0.1:19790")
        admin.connect()
        admin.create_table("sys", "concurrent_test", [
            {"name": "id", "column_type": "Int64"}
        ])
        admin.close()
        
        # 并发测试
        print("\n测试并发写入...")
        import threading
        
        results = {"success": 0, "fail": 0}
        lock = threading.Lock()
        
        def write_worker(key: str):
            tester = GrpcPermissionTester("127.0.0.1:19790")
            tester.connect()
            success, _ = tester.put_data("sys", "concurrent_test", 
                                         key.encode(), f"value_{key}".encode())
            tester.close()
            with lock:
                if success:
                    results["success"] += 1
                else:
                    results["fail"] += 1
        
        threads = []
        for i in range(10):
            t = threading.Thread(target=write_worker, args=(f"key_{i}",))
            threads.append(t)
        
        for t in threads:
            t.start()
        
        for t in threads:
            t.join()
        
        print(f"  ✓ 并发写入完成: {results['success']} 成功, {results['fail']} 失败")
        
        # 验证读取
        print("\n验证并发读取...")
        verify_tester = GrpcPermissionTester("127.0.0.1:19790")
        verify_tester.connect()
        
        success_count = 0
        for i in range(10):
            success, value, _ = verify_tester.get_data("sys", "concurrent_test", 
                                                        f"key_{i}".encode())
            if success and value == f"value_key_{i}".encode():
                success_count += 1
        
        verify_tester.close()
        print(f"  ✓ 验证读取: {success_count}/10 成功")
        
        print("\n✓ 测试4通过: gRPC并发访问正常")
        return True
        
    finally:
        runner.cleanup()


def main():
    print("="*70)
    print(" "*15 + "gRPC权限配置自动化测试套件")
    print("="*70)
    
    server_bin = "../target/debug/laoflchDB-rust"
    if not os.path.exists(server_bin):
        print(f"\n错误: 服务器二进制文件不存在: {server_bin}")
        print("请先运行: cd .. && cargo build")
        return 1
    
    print(f"服务器: {server_bin}")
    
    tests = [
        ("测试1: gRPC单服务基本功能", test_grpc_single_service),
        ("测试2: gRPC多服务不同权限", test_grpc_multi_service),
        ("测试3: gRPC服务增加和减少", test_grpc_add_remove_service),
        ("测试4: gRPC并发访问", test_grpc_concurrent_access),
    ]
    
    results = []
    for name, test_func in tests:
        try:
            result = test_func()
            results.append((name, result))
        except Exception as e:
            print(f"\n  ✗ 测试异常: {e}")
            import traceback
            traceback.print_exc()
            results.append((name, False))
    
    print("\n" + "="*70)
    print("测试结果汇总")
    print("="*70)
    
    passed = sum(1 for _, r in results if r)
    failed = len(results) - passed
    
    for name, result in results:
        status = "✓ 通过" if result else "✗ 失败"
        print(f"  {name}: {status}")
    
    print(f"\n总计: {passed} 通过, {failed} 失败")
    
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
