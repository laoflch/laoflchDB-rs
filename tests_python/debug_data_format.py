#!/usr/bin/env python3
import requests
import json
import base64

BASE_URL = "http://127.0.0.1:8080"

# 插入测试数据
print("=== 插入测试数据 ===")
payload = {
    "schema": "sys",
    "table": "test_data_validation",
    "row": {
        "row_type": 1,
        "version": 1,
        "data": [
            base64.b64encode(b"1").decode(),
            base64.b64encode(b"TestName").decode(),
            base64.b64encode(b"test@example.com").decode(),
            base64.b64encode(b"25").decode(),
            base64.b64encode(b"95.5").decode(),
        ]
    }
}
resp = requests.post(f"{BASE_URL}/api/v1/schemas/sys/tables/test_data_validation/rows", json=payload)
print(f"插入响应: {resp.text}")

# 查询数据
print("\n=== 查询数据 ===")
resp = requests.post(f"{BASE_URL}/api/v1/sql_query", json={"sql": "SELECT * FROM test_data_validation"})
print(f"查询响应: {resp.text}")

# 解析数据
try:
    data = resp.json()
    print("\n=== 解析数据 ===")
    print(f"列名: {data['data']['columns']}")
    print(f"行数: {len(data['data']['rows'])}")
    for i, row in enumerate(data['data']['rows']):
        print(f"\n行{i+1}: {row}")
        for j, val in enumerate(row):
            if isinstance(val, str) and "=" in val:
                try:
                    decoded = base64.b64decode(val).decode()
                    print(f"  列{j} ({data['data']['columns'][j]}): base64={val} -> 解码后='{decoded}'")
                except:
                    print(f"  列{j} ({data['data']['columns'][j]}): base64={val} -> 解码失败")
            else:
                print(f"  列{j} ({data['data']['columns'][j]}): '{val}'")
except Exception as e:
    print(f"解析失败: {e}")
