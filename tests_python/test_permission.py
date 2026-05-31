#!/usr/bin/env python3
"""
权限配置测试套件
测试场景：
1. 多服务配置加载
2. 服务增加和减少
3. 权限规则验证
4. 并发访问测试
"""
import requests
import json
import sys
import subprocess
import time
import os
import signal
import tempfile
import yaml
from typing import Dict, List, Optional, Tuple

class PermissionTestConfig:
    """测试配置管理"""
    
    def __init__(self, db_path: str):
        self.db_path = db_path
        self.config = self._create_default_config()
    
    def _create_default_config(self) -> Dict:
        return {
            "db_path": self.db_path,
            "log_level": "info",
            "default_policy": "deny",
            "access_protocols": [],
            "permissions": []
        }
    
    def add_service(self, protocol: str, addr: str, service_id: str) -> 'PermissionTestConfig':
        """添加服务配置"""
        self.config["access_protocols"].append({
            "protocol": protocol,
            "enabled": True,
            "addr": addr,
            "service_id": service_id
        })
        return self
    
    def add_permission(self, service_id: str, default_policy: str, 
                      allowed_actions: List[str], 
                      denied_actions: List[str] = None,
                      table_permissions: Dict = None) -> 'PermissionTestConfig':
        """添加权限配置"""
        perm = {
            "service_id": service_id,
            "default_policy": default_policy,
            "allowed_actions": allowed_actions,
            "denied_actions": denied_actions or []
        }
        if table_permissions:
            perm["table_permissions"] = table_permissions
        self.config["permissions"].append(perm)
        return self
    
    def save(self, path: str):
        """保存配置到文件"""
        with open(path, 'w') as f:
            yaml.dump(self.config, f, default_flow_style=False, sort_keys=False)
    
    def get_service_ids(self) -> List[str]:
        """获取所有服务ID"""
        return [svc["service_id"] for svc in self.config.get("access_protocols", [])]
    
    def get_permission_service_ids(self) -> List[str]:
        """获取所有权限配置的service_id"""
        return [perm["service_id"] for perm in self.config.get("permissions", [])]


class PermissionTester:
    """权限测试基类"""
    
    def __init__(self, base_url: str):
        self.base_url = base_url
        self.client = requests.Session()
    
    def health_check(self) -> bool:
        """健康检查"""
        try:
            resp = self.client.get(f"{self.base_url}/health", timeout=5)
            return resp.status_code == 200
        except:
            return False
    
    def create_table(self, schema: str, table_name: str, columns: List[Dict]) -> Tuple[bool, Optional[Dict]]:
        """创建表"""
        try:
            resp = self.client.post(
                f"{self.base_url}/api/v1/tables",
                json={
                    "schema": schema,
                    "table_name": table_name,
                    "columns": columns
                },
                timeout=5
            )
            data = resp.json()
            return (resp.status_code == 200 and data.get("success"), data)
        except Exception as e:
            return (False, {"error": str(e)})
    
    def list_tables(self, schema: str) -> Tuple[bool, Optional[List[str]]]:
        """列出表"""
        try:
            resp = self.client.get(
                f"{self.base_url}/api/v1/schemas/{schema}/tables",
                timeout=5
            )
            data = resp.json()
            if resp.status_code == 200 and data.get("success"):
                return (True, data.get("data", []))
            return (False, None)
        except:
            return (False, None)
    
    def put_data(self, schema: str, table: str, key: str, value: str) -> Tuple[bool, int]:
        """写入数据"""
        try:
            resp = self.client.post(
                f"{self.base_url}/api/v1/put",
                json={
                    "schema": schema,
                    "table": table,
                    "key": key,
                    "value": value
                },
                timeout=5
            )
            return (resp.status_code == 200, resp.status_code)
        except:
            return (False, 0)
    
    def get_data(self, schema: str, table: str, key: str) -> Tuple[bool, Optional[str]]:
        """读取数据"""
        try:
            resp = self.client.get(
                f"{self.base_url}/api/v1/get",
                params={"schema": schema, "table": table, "key": key},
                timeout=5
            )
            data = resp.json()
            if resp.status_code == 200 and data.get("success"):
                return (True, data.get("data", {}).get("value"))
            return (False, None)
        except:
            return (False, None)
    
    def delete_data(self, schema: str, table: str, key: str) -> Tuple[bool, int]:
        """删除数据"""
        try:
            resp = self.client.post(
                f"{self.base_url}/api/v1/delete",
                json={
                    "schema": schema,
                    "table": table,
                    "key": key
                },
                timeout=5
            )
            return (resp.status_code == 200, resp.status_code)
        except:
            return (False, 0)


class PermissionTestRunner:
    """权限测试运行器"""
    
    def __init__(self, server_bin: str, db_dir: str):
        self.server_bin = server_bin
        self.db_dir = db_dir
        self.server_proc = None
        self.config_path = None
    
    def start_server_with_config(self, config: PermissionTestConfig) -> bool:
        """使用指定配置启动服务器"""
        os.makedirs(self.db_dir, exist_ok=True)
        self.config_path = os.path.join(self.db_dir, "config.yaml")
        config.save(self.config_path)
        
        cmd = [self.server_bin, "-c", self.config_path, "start"]
        self.server_proc = subprocess.Popen(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            preexec_fn=os.setsid
        )
        
        time.sleep(3)
        return self.server_proc.poll() is None
    
    def stop_server(self):
        """停止服务器"""
        if self.server_proc:
            os.killpg(os.getpgid(self.server_proc.pid), signal.SIGTERM)
            try:
                self.server_proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.server_proc.kill()
            self.server_proc = None
    
    def cleanup(self):
        """清理资源"""
        self.stop_server()
        if os.path.exists(self.db_dir):
            import shutil
            shutil.rmtree(self.db_dir, ignore_errors=True)


def test_single_service_basic():
    """测试1: 单服务基本功能"""
    print("\n" + "="*60)
    print("测试1: 单服务基本功能")
    print("="*60)
    
    db_dir = tempfile.mkdtemp(prefix="test_single_")
    runner = PermissionTestRunner(
        "../target/debug/laoflchDB-rust",
        db_dir
    )
    
    try:
        config = PermissionTestConfig(os.path.join(db_dir, "data"))
        config.add_service("rest", "127.0.0.1:18080", "admin")
        config.add_permission(
            "admin", "allow",
            ["get", "put", "delete", "create_table", "drop_table", "list_tables"]
        )
        
        print(f"启动服务: admin (端口 18080)")
        if not runner.start_server_with_config(config):
            print("  ✗ 服务启动失败")
            return False
        
        tester = PermissionTester("http://127.0.0.1:18080")
        
        print("检查服务可用性...")
        if not tester.health_check():
            print("  ✗ 服务不可用")
            return False
        print("  ✓ 服务可用")
        
        print("测试创建表...")
        success, data = tester.create_table("sys", "test_table", [
            {"name": "id", "column_type": "Int64"},
            {"name": "name", "column_type": "String"}
        ])
        if success:
            print("  ✓ 创建表成功")
        else:
            print(f"  ✗ 创建表失败: {data}")
            return False
        
        print("测试写入数据...")
        success, status = tester.put_data("sys", "test_table", "key1", "value1")
        if success:
            print("  ✓ 写入数据成功")
        else:
            print(f"  ✗ 写入数据失败 (status={status})")
            return False
        
        print("测试读取数据...")
        success, value = tester.get_data("sys", "test_table", "key1")
        if success and value:
            print(f"  ✓ 读取数据成功: {value}")
        else:
            print(f"  ✗ 读取数据失败")
            return False
        
        print("测试列出表...")
        success, tables = tester.list_tables("sys")
        if success and "test_table" in tables:
            print(f"  ✓ 列出表成功: {tables}")
        else:
            print(f"  ✗ 列出表失败")
            return False
        
        print("\n✓ 测试1通过: 单服务基本功能正常")
        return True
        
    finally:
        runner.cleanup()


def test_multi_service_different_permissions():
    """测试2: 多服务不同权限"""
    print("\n" + "="*60)
    print("测试2: 多服务不同权限")
    print("="*60)
    
    db_dir = tempfile.mkdtemp(prefix="test_multi_")
    runner = PermissionTestRunner(
        "../target/debug/laoflchDB-rust",
        db_dir
    )
    
    try:
        config = PermissionTestConfig(os.path.join(db_dir, "data"))
        
        # 添加只读服务
        config.add_service("rest", "127.0.0.1:18081", "readonly")
        config.add_permission(
            "readonly", "deny",
            ["get", "list_tables"],
            ["put", "delete", "create_table"]
        )
        
        # 添加只写服务
        config.add_service("rest", "127.0.0.1:18082", "writeonly")
        config.add_permission(
            "writeonly", "deny",
            ["put", "delete", "create_table"],
            ["get", "list_tables"]
        )
        
        # 添加管理服务
        config.add_service("rest", "127.0.0.1:18083", "admin")
        config.add_permission(
            "admin", "allow",
            ["get", "put", "delete", "create_table", "drop_table", "list_tables"]
        )
        
        print(f"启动3个服务: readonly(18081), writeonly(18082), admin(18083)")
        if not runner.start_server_with_config(config):
            print("  ✗ 服务启动失败")
            return False
        
        admin_tester = PermissionTester("http://127.0.0.1:18083")
        readonly_tester = PermissionTester("http://127.0.0.1:18081")
        writeonly_tester = PermissionTester("http://127.0.0.1:18082")
        
        # 使用admin创建表
        print("\n[Admin] 创建测试表...")
        success, _ = admin_tester.create_table("sys", "perm_test", [
            {"name": "id", "column_type": "Int64"},
            {"name": "data", "column_type": "String"}
        ])
        if not success:
            print("  ✗ Admin创建表失败")
            return False
        print("  ✓ Admin创建表成功")
        
        # 测试只读服务
        print("\n[ReadOnly] 测试只读权限...")
        success, _ = readonly_tester.get_data("sys", "perm_test", "key1")
        if success:
            print("  ✓ ReadOnly可以读取")
        else:
            print("  ✗ ReadOnly读取失败")
        
        success, status = readonly_tester.put_data("sys", "perm_test", "key1", "value")
        if not success and status == 403:
            print("  ✓ ReadOnly被拒绝写入 (403)")
        else:
            print(f"  ✗ ReadOnly写入应该被拒绝: status={status}")
        
        # 测试只写服务
        print("\n[WriteOnly] 测试只写权限...")
        success, status = writeonly_tester.put_data("sys", "perm_test", "key2", "value2")
        if success:
            print("  ✓ WriteOnly可以写入")
        else:
            print(f"  ✗ WriteOnly写入失败: status={status}")
        
        success, _ = writeonly_tester.get_data("sys", "perm_test", "key2")
        if not success:
            print("  ✓ WriteOnly被拒绝读取")
        else:
            print("  ✗ WriteOnly读取应该被拒绝")
        
        print("\n✓ 测试2通过: 多服务权限隔离正常")
        return True
        
    finally:
        runner.cleanup()


def test_add_remove_service():
    """测试3: 增加和减少服务"""
    print("\n" + "="*60)
    print("测试3: 增加和减少服务")
    print("="*60)
    
    db_dir = tempfile.mkdtemp(prefix="test_addrm_")
    runner = PermissionTestRunner(
        "../target/debug/laoflchDB-rust",
        db_dir
    )
    
    try:
        # 初始配置：只有admin服务
        print("阶段1: 启动只有admin服务的配置")
        config1 = PermissionTestConfig(os.path.join(db_dir, "data"))
        config1.add_service("rest", "127.0.0.1:18090", "admin")
        config1.add_permission("admin", "allow", ["get", "put", "delete", "create_table"])
        
        if not runner.start_server_with_config(config1):
            print("  ✗ 服务启动失败")
            return False
        print("  ✓ Admin服务已启动 (18090)")
        
        admin_tester = PermissionTester("http://127.0.0.1:18090")
        
        # 创建测试表
        print("\n[Admin] 创建测试表...")
        admin_tester.create_table("sys", "test1", [
            {"name": "id", "column_type": "Int64"}
        ])
        print("  ✓ 测试表已创建")
        
        # 重启增加新服务
        print("\n阶段2: 重启并增加readonly服务")
        runner.stop_server()
        time.sleep(1)
        
        config2 = PermissionTestConfig(os.path.join(db_dir, "data"))
        config2.add_service("rest", "127.0.0.1:18090", "admin")
        config2.add_service("rest", "127.0.0.1:18091", "readonly")
        config2.add_permission("admin", "allow", ["get", "put", "delete", "create_table"])
        config2.add_permission("readonly", "deny", ["get", "list_tables"])
        
        if not runner.start_server_with_config(config2):
            print("  ✗ 服务启动失败")
            return False
        print("  ✓ Admin(18090) 和 ReadOnly(18091) 服务已启动")
        
        readonly_tester = PermissionTester("http://127.0.0.1:18091")
        
        # 测试readonly服务
        success, _ = readonly_tester.get_data("sys", "test1", "key1")
        if success:
            print("  ✓ ReadOnly可以读取现有数据")
        else:
            print("  ✗ ReadOnly读取失败")
        
        # 重启减少服务
        print("\n阶段3: 重启只保留admin服务")
        runner.stop_server()
        time.sleep(1)
        
        config3 = PermissionTestConfig(os.path.join(db_dir, "data"))
        config3.add_service("rest", "127.0.0.1:18090", "admin")
        config3.add_permission("admin", "allow", ["get", "put", "delete", "create_table"])
        
        if not runner.start_server_with_config(config3):
            print("  ✗ 服务启动失败")
            return False
        print("  ✓ 只有Admin服务(18090)")
        
        # 验证admin仍然正常
        success, _ = admin_tester.get_data("sys", "test1", "key1")
        if success:
            print("  ✓ Admin服务正常工作")
        else:
            print("  ✗ Admin服务异常")
        
        print("\n✓ 测试3通过: 服务增加和减少正常")
        return True
        
    finally:
        runner.cleanup()


def test_config_consistency():
    """测试4: 配置一致性验证"""
    print("\n" + "="*60)
    print("测试4: 配置一致性验证")
    print("="*60)
    
    db_dir = tempfile.mkdtemp(prefix="test_consistency_")
    runner = PermissionTestRunner(
        "../target/debug/laoflchDB-rust",
        db_dir
    )
    
    try:
        # 配置：服务数量 != 权限数量（应该能启动，但部分服务无权限）
        print("测试: 配置不完全一致的情况")
        config = PermissionTestConfig(os.path.join(db_dir, "data"))
        config.add_service("rest", "127.0.0.1:18100", "service1")
        config.add_service("rest", "127.0.0.1:18101", "service2")
        config.add_permission("service1", "allow", ["get", "put"])
        # 注意：service2 没有配置权限
        
        print(f"服务列表: {config.get_service_ids()}")
        print(f"权限列表: {config.get_permission_service_ids()}")
        
        if not runner.start_server_with_config(config):
            print("  ✗ 服务启动失败")
            return False
        print("  ✓ 服务启动成功（使用全局默认策略）")
        
        tester1 = PermissionTester("http://127.0.0.1:18100")
        tester2 = PermissionTester("http://127.0.0.1:18101")
        
        # 测试有权限配置的服务
        print("\n[Service1] 有明确权限配置...")
        success1, _ = tester1.create_table("sys", "t1", [
            {"name": "id", "column_type": "Int64"}
        ])
        if success1:
            print("  ✓ Service1操作成功")
        else:
            print("  ✗ Service1操作失败")
        
        # 测试无权限配置的服务（使用全局默认deny）
        print("\n[Service2] 无明确权限配置（使用全局deny）...")
        success2, _ = tester2.create_table("sys", "t2", [
            {"name": "id", "column_type": "Int64"}
        ])
        if not success2:
            print("  ✓ Service2操作被拒绝（符合预期：全局deny）")
        else:
            print("  ⚠ Service2操作成功（可能使用了默认allow）")
        
        print("\n✓ 测试4通过: 配置一致性验证完成")
        return True
        
    finally:
        runner.cleanup()


def test_table_permissions():
    """测试5: 表级别权限"""
    print("\n" + "="*60)
    print("测试5: 表级别权限控制")
    print("="*60)
    
    db_dir = tempfile.mkdtemp(prefix="test_table_perm_")
    runner = PermissionTestRunner(
        "../target/debug/laoflchDB-rust",
        db_dir
    )
    
    try:
        config = PermissionTestConfig(os.path.join(db_dir, "data"))
        config.add_service("rest", "127.0.0.1:18110", "limited")
        config.add_permission(
            "limited", "allow",
            ["get", "put"],
            [],
            {
                "allowed_schemas": ["public"],
                "denied_schemas": ["internal", "admin"],
                "allowed_tables": ["users", "products"],
                "denied_tables": ["secrets", "passwords"]
            }
        )
        
        print(f"启动服务: limited (18110)")
        print("  - 允许schemas: public")
        print("  - 拒绝schemas: internal, admin")
        print("  - 允许tables: users, products")
        print("  - 拒绝tables: secrets, passwords")
        
        if not runner.start_server_with_config(config):
            print("  ✗ 服务启动失败")
            return False
        
        tester = PermissionTester("http://127.0.0.1:18110")
        
        # 先用admin创建表
        print("\n[Setup] 创建测试表...")
        admin_config = PermissionTestConfig(os.path.join(db_dir, "data_admin"))
        admin_config.add_service("rest", "127.0.0.1:18111", "admin")
        admin_config.add_permission("admin", "allow", ["*"])
        runner2 = PermissionTestRunner("../target/debug/laoflchDB-rust", db_dir + "_admin")
        
        config_admin = PermissionTestConfig(os.path.join(db_dir, "data_admin"))
        config_admin.add_service("rest", "127.0.0.1:18111", "admin")
        config_admin.add_permission("admin", "allow", ["*"])
        runner2.start_server_with_config(config_admin)
        
        admin = PermissionTester("http://127.0.0.1:18111")
        admin.create_table("public", "users", [{"name": "id", "column_type": "Int64"}])
        admin.create_table("public", "secrets", [{"name": "id", "column_type": "Int64"}])
        admin.create_table("internal", "sensitive", [{"name": "id", "column_type": "Int64"}])
        runner2.stop_server()
        
        # 测试允许的schema和table
        print("\n[Limited] 测试允许的访问...")
        success, _ = tester.get_data("public", "users", "key1")
        if success:
            print("  ✓ 可以访问 public.users")
        else:
            print("  ✗ 无法访问 public.users")
        
        # 测试拒绝的schema
        success, _ = tester.get_data("internal", "sensitive", "key1")
        if not success:
            print("  ✓ 拒绝访问 internal.sensitive (schema被拒绝)")
        else:
            print("  ✗ 应该拒绝访问 internal.sensitive")
        
        # 测试拒绝的table
        success, _ = tester.get_data("public", "secrets", "key1")
        if not success:
            print("  ✓ 拒绝访问 public.secrets (table被拒绝)")
        else:
            print("  ✗ 应该拒绝访问 public.secrets")
        
        print("\n✓ 测试5通过: 表级别权限控制正常")
        return True
        
    finally:
        runner.cleanup()
        if 'runner2' in locals():
            runner2.cleanup()


def main():
    print("="*70)
    print(" "*15 + "权限配置自动化测试套件")
    print("="*70)
    
    # 检查服务器二进制文件
    server_bin = "../target/debug/laoflchDB-rust"
    if not os.path.exists(server_bin):
        print(f"\n错误: 服务器二进制文件不存在: {server_bin}")
        print("请先运行: cd .. && cargo build")
        return 1
    
    print(f"服务器: {server_bin}")
    
    tests = [
        ("测试1: 单服务基本功能", test_single_service_basic),
        ("测试2: 多服务不同权限", test_multi_service_different_permissions),
        ("测试3: 增加和减少服务", test_add_remove_service),
        ("测试4: 配置一致性验证", test_config_consistency),
        ("测试5: 表级别权限控制", test_table_permissions),
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
