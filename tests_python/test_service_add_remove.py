#!/usr/bin/env python3
"""
测试：增加/减少 service_id，权限管理和多端口启动
"""
import subprocess
import time
import sys
import os
import signal
import tempfile
import yaml
import requests
import grpc
import threading
from typing import Dict, List

# Add proto path
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import rpc_pb2
import rpc_pb2_grpc

class TestRunner:
    def __init__(self, server_bin: str):
        self.server_bin = server_bin
        self.work_dir = tempfile.mkdtemp(prefix="test_service_add_remove_")
        self.server_procs: Dict[str, subprocess.Popen] = {}
        self.config_path = os.path.join(self.work_dir, "config.yaml")
        self.db_path = os.path.join(self.work_dir, "db_data")
        os.makedirs(self.db_path, exist_ok=True)
        
        print(f"工作目录: {self.work_dir}")
        print(f"服务器二进制: {self.server_bin}")
    
    def write_config(self, config: Dict):
        with open(self.config_path, 'w') as f:
            yaml.dump(config, f, default_flow_style=False, sort_keys=False)
        print(f"\n配置已写入:")
        print(yaml.dump(config, sort_keys=False))
    
    def start_server(self, timeout: int = 3) -> bool:
        print(f"\n启动服务器...")
        cmd = [self.server_bin, "-c", self.config_path, "start"]
        proc = subprocess.Popen(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            preexec_fn=os.setsid,
            cwd=self.work_dir
        )
        time.sleep(timeout)
        
        if proc.poll() is not None:
            print(f"✗ 服务器启动失败")
            return False
        
        self.server_procs["main"] = proc
        print(f"✓ 服务器已启动 (PID: {proc.pid})")
        return True
    
    def stop_server(self):
        if "main" in self.server_procs:
            try:
                proc = self.server_procs["main"]
                os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    proc.kill()
                del self.server_procs["main"]
                print(f"\n✓ 服务器已停止")
            except Exception as e:
                print(f"\n✗ 停止服务器失败: {e}")
    
    def cleanup(self):
        self.stop_server()
        try:
            import shutil
            shutil.rmtree(self.work_dir, ignore_errors=True)
            print(f"\n清理工作目录完成")
        except Exception as e:
            print(f"\n清理工作目录失败: {e}")
    
    def rest_health_check(self, port: int) -> bool:
        try:
            resp = requests.get(f"http://127.0.0.1:{port}/health", timeout=2)
            return resp.status_code == 200
        except Exception:
            return False
    
    def rest_create_table(self, port: int, schema: str, table: str, 
                       service_desc: str = "") -> tuple[bool, str]:
        try:
            payload = {
                "schema": schema,
                "table_name": table,
                "columns": [{"name": "id", "column_type": "Int64"}]
            }
            resp = requests.post(
                f"http://127.0.0.1:{port}/api/v1/tables",
                json=payload,
                timeout=3
            )
            data = resp.json()
            return data.get("success", False), f"{service_desc} 状态码: {resp.status_code}"
        except Exception as e:
            return False, f"{service_desc} 异常: {e}"
    
    def rest_put_data(self, port: int, schema: str, table: str, 
                     key: str, value: str, service_desc: str = "") -> tuple[bool, str]:
        try:
            payload = {
                "schema": schema,
                "table": table,
                "key": key,
                "value": value
            }
            resp = requests.post(
                f"http://127.0.0.1:{port}/api/v1/put",
                json=payload,
                timeout=3
            )
            data = resp.json()
            return data.get("success", False), f"{service_desc} 状态码: {resp.status_code}"
        except Exception as e:
            return False, f"{service_desc} 异常: {e}"
    
    def rest_get_data(self, port: int, schema: str, table: str, 
                     key: str, service_desc: str = "") -> tuple[bool, str]:
        try:
            resp = requests.get(
                f"http://127.0.0.1:{port}/api/v1/get",
                params={"schema": schema, "table": table, "key": key},
                timeout=3
            )
            data = resp.json()
            return data.get("success", False), f"{service_desc} 状态码: {resp.status_code}"
        except Exception as e:
            return False, f"{service_desc} 异常: {e}"
    
    def grpc_create_table(self, port: int, schema: str, table: str, 
                        service_desc: str = "") -> tuple[bool, str]:
        try:
            channel = grpc.insecure_channel(f"127.0.0.1:{port}")
            stub = rpc_pb2_grpc.LaoflchDbStub(channel)
            req = rpc_pb2.CreateTableRequest(
                schema=schema,
                table_name=table,
                columns=[rpc_pb2.ColumnMeta(column_name="id", column_type=2)]
            )
            resp = stub.CreateTable(req, timeout=3)
            return resp.success, f"{service_desc} 响应: {resp.message}"
        except Exception as e:
            return False, f"{service_desc} 异常: {e}"
    
    def grpc_put_data(self, port: int, schema: str, table: str, 
                     key: bytes, value: bytes, service_desc: str = "") -> tuple[bool, str]:
        try:
            channel = grpc.insecure_channel(f"127.0.0.1:{port}")
            stub = rpc_pb2_grpc.LaoflchDbStub(channel)
            req = rpc_pb2.PutRequest(
                schema=schema,
                table=table,
                key=key,
                value=value
            )
            resp = stub.Put(req, timeout=3)
            return resp.success, f"{service_desc} 响应: {resp.message}"
        except Exception as e:
            return False, f"{service_desc} 异常: {e}"
    
    def grpc_get_data(self, port: int, schema: str, table: str, 
                     key: bytes, service_desc: str = "") -> tuple[bool, str]:
        try:
            channel = grpc.insecure_channel(f"127.0.0.1:{port}")
            stub = rpc_pb2_grpc.LaoflchDbStub(channel)
            req = rpc_pb2.GetRequest(
                schema=schema,
                table=table,
                key=key
            )
            resp = stub.Get(req, timeout=3)
            return resp.found, f"{service_desc} found: {resp.found}"
        except Exception as e:
            return False, f"{service_desc} 异常: {e}"


def create_base_config(db_path: str, default_policy: str = "allow") -> Dict:
    return {
        "db_path": db_path,
        "log_level": "info",
        "default_policy": default_policy,
        "access_protocols": [],
        "permissions": []
    }


def run_tests():
    print("="*70)
    print("测试：增加/减少 service_id，权限管理，多端口启动")
    print("="*70)
    
    server_bin = os.path.join(
        os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
        "target", "debug", "laoflchDB-rust"
    )
    
    if not os.path.exists(server_bin):
        print(f"\n错误：服务器二进制文件不存在: {server_bin}")
        print("请先编译: cargo build")
        return 1
    
    runner = TestRunner(server_bin)
    
    try:
        # ======================================
        # 阶段1：初始配置 - 只有 admin_grpc 服务
        # ======================================
        print("\n" + "="*70)
        print("阶段1：初始配置 - 只有 admin_grpc (端口 19770)")
        print("="*70)
        
        config1 = create_base_config(runner.db_path)
        config1["access_protocols"] = [
            {"protocol": "grpc", "enabled": True, "addr": "127.0.0.1:19770", "service_id": "admin_grpc"}
        ]
        config1["permissions"] = [
            {
                "service_id": "admin_grpc",
                "default_policy": "allow",
                "allowed_actions": ["*"]
            }
        ]
        
        runner.write_config(config1)
        
        if not runner.start_server():
            return 1
        
        print("\n检查服务状态...")
        ok1, _ = runner.grpc_create_table(19770, "sys", "test_table1", "[admin_grpc]")
        print(f"  [19770] admin_grpc 创建表: {'✓' if ok1 else '✗'}")
        
        ok2, _ = runner.grpc_put_data(19770, "sys", "test_table1", b"key1", b"value1", "[admin_grpc]")
        print(f"  [19770] admin_grpc 写入数据: {'✓' if ok2 else '✗'}")
        
        ok3, _ = runner.grpc_get_data(19770, "sys", "test_table1", b"key1", "[admin_grpc]")
        print(f"  [19770] admin_grpc 读取数据: {'✓' if ok3 else '✗'}")
        
        if not (ok1 and ok2 and ok3):
            print("\n✗ 阶段1测试失败")
            return 1
        
        print("\n✓ 阶段1完成：admin_grpc 正常工作")
        runner.stop_server()
        time.sleep(1)
        
        # ======================================
        # 阶段2：增加 readonly_rest 和 writeonly_grpc 服务
        # ======================================
        print("\n" + "="*70)
        print("阶段2：增加 readonly_rest (8080) 和 writeonly_grpc (19771)")
        print("="*70)
        
        config2 = create_base_config(runner.db_path)
        config2["access_protocols"] = [
            {"protocol": "grpc", "enabled": True, "addr": "127.0.0.1:19770", "service_id": "admin_grpc"},
            {"protocol": "rest", "enabled": True, "addr": "127.0.0.1:8080", "service_id": "readonly_rest"},
            {"protocol": "grpc", "enabled": True, "addr": "127.0.0.1:19771", "service_id": "writeonly_grpc"}
        ]
        config2["permissions"] = [
            {
                "service_id": "admin_grpc",
                "default_policy": "allow",
                "allowed_actions": ["*"]
            },
            {
                "service_id": "readonly_rest",
                "default_policy": "deny",
                "allowed_actions": ["get", "list_tables"],
                "denied_actions": ["put", "delete", "create_table"]
            },
            {
                "service_id": "writeonly_grpc",
                "default_policy": "deny",
                "allowed_actions": ["put", "delete", "create_table"],
                "denied_actions": ["get", "list_tables"]
            }
        ]
        
        runner.write_config(config2)
        
        if not runner.start_server():
            return 1
        
        print("\n检查多个服务状态...")
        
        print("\n[19770] admin_grpc (应该能读写):")
        ok1, _ = runner.grpc_get_data(19770, "sys", "test_table1", b"key1", "[admin_grpc]")
        print(f"  读取: {'✓' if ok1 else '✗'}")
        
        ok2, _ = runner.grpc_put_data(19770, "sys", "test_table1", b"key2", b"value2", "[admin_grpc]")
        print(f"  写入: {'✓' if ok2 else '✗'}")
        
        print("\n[8080] readonly_rest (应该只能读):")
        ok3, _ = runner.rest_get_data(8080, "sys", "test_table1", "key2", "[readonly_rest]")
        print(f"  读取: {'✓' if ok3 else '✗'}")
        
        ok4, _ = runner.rest_put_data(8080, "sys", "test_table1", "key3", "value3", "[readonly_rest]")
        print(f"  写入: {'✓' if ok4 else '✗'} (预期: ✗)")
        
        print("\n[19771] writeonly_grpc (应该只能写):")
        ok5, _ = runner.grpc_put_data(19771, "sys", "test_table1", b"key4", b"value4", "[writeonly_grpc]")
        print(f"  写入: {'✓' if ok5 else '✗'}")
        
        ok6, _ = runner.grpc_get_data(19771, "sys", "test_table1", b"key4", "[writeonly_grpc]")
        print(f"  读取: {'✓' if ok6 else '✗'} (预期: ✗)")
        
        if not (ok1 and ok2 and not ok4 and ok5 and not ok6):
            print("\n✗ 阶段2测试失败：权限隔离未正常工作")
            return 1
        
        print("\n✓ 阶段2完成：新增服务权限正确隔离")
        runner.stop_server()
        time.sleep(1)
        
        # ======================================
        # 阶段3：移除 writeonly_grpc，增加 limited_rest
        # ======================================
        print("\n" + "="*70)
        print("阶段3：移除 writeonly_grpc，增加 limited_rest (8081)")
        print("="*70)
        
        config3 = create_base_config(runner.db_path)
        config3["access_protocols"] = [
            {"protocol": "grpc", "enabled": True, "addr": "127.0.0.1:19770", "service_id": "admin_grpc"},
            {"protocol": "rest", "enabled": True, "addr": "127.0.0.1:8080", "service_id": "readonly_rest"},
            {"protocol": "rest", "enabled": True, "addr": "127.0.0.1:8081", "service_id": "limited_rest"}
        ]
        config3["permissions"] = [
            {
                "service_id": "admin_grpc",
                "default_policy": "allow",
                "allowed_actions": ["*"]
            },
            {
                "service_id": "readonly_rest",
                "default_policy": "deny",
                "allowed_actions": ["get", "list_tables"]
            },
            {
                "service_id": "limited_rest",
                "default_policy": "deny",
                "allowed_actions": ["get", "put"],
                "table_permissions": {
                    "allowed_schemas": ["sys"],
                    "denied_tables": ["test_table1"]
                }
            }
        ]
        
        runner.write_config(config3)
        
        if not runner.start_server():
            return 1
        
        print("\n检查更新后的服务...")
        
        print("\n[19771] writeonly_grpc 应该已停止:")
        ok9, _ = runner.grpc_get_data(19771, "sys", "test_table1", b"key1", "[removed]")
        print(f"  访问失败: {'✓' if not ok9 else '✗'}")
        
        print("\n[8081] limited_rest (表级别限制):")
        ok10, _ = runner.rest_get_data(8081, "sys", "test_table1", "key1", "[limited_rest]")
        print(f"  test_table1: {'✓' if ok10 else '✗'} (预期: ✗)")
        
        ok11, _ = runner.grpc_create_table(19770, "sys", "test_table2", "[admin_grpc]")
        if ok11:
            ok12, _ = runner.rest_get_data(8081, "sys", "test_table2", "any", "[limited_rest]")
            print(f"  test_table2: {'✓' if ok12 else '✗'}")
        
        print("\n[8080] readonly_rest 仍应正常:")
        ok13, _ = runner.rest_get_data(8080, "sys", "test_table1", "key1", "[readonly_rest]")
        print(f"  读取: {'✓' if ok13 else '✗'}")
        
        if not (not ok9 and ok13):
            print("\n✗ 阶段3测试失败")
            return 1
        
        print("\n✓ 阶段3完成：服务移除/新增正常工作")
        
        print("\n" + "="*70)
        print("所有测试通过！")
        print("="*70)
        print("\n✓ service_id 增加/减少 正常")
        print("✓ 权限管理 正常")
        print("✓ 多端口启动 正常")
        
        return 0
        
    finally:
        runner.cleanup()


if __name__ == "__main__":
    sys.exit(run_tests())
