#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""laoflchdb Raft 集群管理客户端（宿主机使用）

通过 gRPC 访问 Raft 节点的状态查询与管理接口。
默认访问 127.0.0.1:29999（部署脚本中 node-1 的宿主映射端口）。

用法示例：
  python3 scripts/raft_admin.py --addr 127.0.0.1:29999 --group 1 status
  python3 scripts/raft_admin.py --addr 127.0.0.1:29999 --group orders status   # group_id 支持字符串
  python3 scripts/raft_admin.py --addr 127.0.0.1:29999 --group 2 list-peers
  python3 scripts/raft_admin.py --addr 127.0.0.1:29999 --group 2 add-peer 2 raft-node-2:29999
  python3 scripts/raft_admin.py --addr 127.0.0.1:29999 --group 2 remove-peer 2
  python3 scripts/raft_admin.py --addr 127.0.0.1:29999 --group 1 put default hello world
  python3 scripts/raft_admin.py --addr 127.0.0.1:29999 --group 1 get default hello
  python3 scripts/raft_admin.py --addr 127.0.0.1:29999 --group 1 delete default hello
  python3 scripts/raft_admin.py --addr 127.0.0.1:29999 --group 1 scan default

依赖：grpcio、grpcio-tools（pip install grpcio grpcio-tools）
"""

import os
import re
import sys
import hashlib
import subprocess
import tempfile
import argparse

PROTO_REL = os.path.join("laoflchdb_raft_service", "proto", "raft_service.proto")
PROTO = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", PROTO_REL)


def _compile_proto(workdir: str) -> str:
    """用 grpc_tools.protoc 编译 raft_service.proto，返回生成目录（加入 sys.path）"""
    try:
        from grpc_tools import protoc
    except ImportError:
        sys.exit("缺少 grpcio-tools，请先执行: pip install grpcio grpcio-tools")

    out_dir = os.path.join(workdir, "gen")
    os.makedirs(out_dir, exist_ok=True)
    proto_dir = os.path.dirname(PROTO)
    rc = protoc.main(
        [
            "grpc_tools.protoc",
            "-I" + proto_dir,
            "--python_out=" + out_dir,
            "--grpc_python_out=" + out_dir,
            os.path.basename(PROTO),
        ]
    )
    if rc != 0:
        sys.exit(f"protoc 编译失败: {rc}")
    return out_dir


def _load_stubs():
    """编译 proto 并导入 stub 模块（grpc_tools 生成平铺的 *_pb2.py 文件）"""
    workdir = tempfile.mkdtemp(prefix="raft_admin_")
    gen = _compile_proto(workdir)
    sys.path.insert(0, gen)

    pb2 = __import__("raft_service_pb2", fromlist=["*"])
    pb2_grpc = __import__("raft_service_pb2_grpc", fromlist=["*"])
    return pb2, pb2_grpc


def _stub(addr: str):
    import grpc

    pb2, pb2_grpc = _load_stubs()
    channel = grpc.insecure_channel(addr)
    svc = pb2_grpc.RaftKvServiceStub(channel)
    return pb2, svc


def _b(payload: str) -> bytes:
    return payload.encode("utf-8")


# ── Rust Debug 文本解析（服务端 GetStatus 返回 RaftMetrics 的 {:?} 文本）──

def _split_top(s: str):
    """按顶层逗号分割 Rust Debug 字符串（跟踪括号深度）"""
    parts = []
    depth = 0
    start = 0
    for i, ch in enumerate(s):
        if ch in "{([":
            depth += 1
        elif ch in "})]":
            depth -= 1
        elif ch == "," and depth == 0:
            parts.append(s[start:i].strip())
            start = i + 1
    parts.append(s[start:].strip())
    return parts


def _find_close(s: str, open_idx: int) -> int:
    """s[open_idx] 为 {/[/(，返回其匹配的 }/]/) 的索引"""
    close = {"{": "}", "[": "]", "(": ")"}[s[open_idx]]
    depth = 0
    for i in range(open_idx, len(s)):
        if s[i] == s[open_idx]:
            depth += 1
        elif s[i] == close:
            depth -= 1
            if depth == 0:
                return i
    return len(s) - 1


def _unwrap(s: str, start: str = "{") -> str:
    """提取 start 到匹配括号之间的内容；若不存在则原样返回"""
    s = s.strip()
    i = s.find(start)
    if i < 0:
        return s
    return s[i + 1:_find_close(s, i)].strip()


def _parse_fields(s: str):
    """把 'key: value, ...' 解析为 dict（value 保留嵌套原始文本）"""
    fields = {}
    for part in _split_top(s):
        if ":" not in part:
            continue
        k, v = part.split(":", 1)
        fields[k.strip()] = v.strip()
    return fields


def _unwrap_some(v: str):
    """'Some(1)' -> '1'；'None' -> None；其他原样返回"""
    v = v.strip()
    if v == "None":
        return None
    if v.startswith("Some("):
        return _unwrap(v, "(")
    return v


def _fmt_logid(v: str) -> str:
    """'LogId { leader_id: LeaderId { term: 1, node_id: 1 }, index: 7 }' -> 'T1-N1-7'"""
    fs = _parse_fields(_unwrap(v, "{"))
    lid = _parse_fields(_unwrap(fs.get("leader_id", ""), "{"))
    return "T{}-N{}-{}".format(
        lid.get("term", "?"), lid.get("node_id", "?"), fs.get("index", "?")
    )


def _fmt_vote(v: str) -> str:
    """'Vote { leader_id: LeaderId { term: 1, node_id: 1 }, committed: true }'"""
    fs = _parse_fields(_unwrap(v, "{"))
    lid = _parse_fields(_unwrap(fs.get("leader_id", ""), "{"))
    state = "committed" if fs.get("committed") == "true" else "uncommitted"
    return "T{}-N{} {}".format(lid.get("term", "?"), lid.get("node_id", "?"), state)


def _print_status(status: str, group_id="1"):
    try:
        metrics = _parse_fields(_unwrap(status, "{"))
        node_id = metrics.get("id", "?")
        role = metrics.get("state", "?")
        running = _unwrap(metrics.get("running_state", "?"), "(")
        # 'Ok(())' -> '()' -> OK；其他错误原样显示
        running = "OK" if running in ("()", "") else running

        # 集群成员（membership_config -> membership -> configs/nodes）
        members = "[]"
        nodes_map = {}
        m_all = _parse_fields(_unwrap(metrics.get("membership_config", ""), "{"))
        mem = _parse_fields(_unwrap(m_all.get("membership", ""), "{"))
        members = mem.get("configs", "[]")
        nodes_raw = mem.get("nodes", "{}")
        for k, v in _parse_fields(_unwrap(nodes_raw, "{")).items():
            addr_fs = _parse_fields(_unwrap(v, "{"))
            nodes_map[k] = addr_fs.get("addr", "?")
        nodes_sorted = sorted(nodes_map.items(), key=lambda x: int(x[0]) if x[0].isdigit() else x[0])

        # 复制进度（replication: Some({2: Some(LogId ...), 3: None})）
        rep = {}
        rep_raw = _unwrap(metrics.get("replication", "None"), "(")
        rep_fields = _parse_fields(_unwrap(rep_raw, "{"))
        for k, v in rep_fields.items():
            if v == "None":
                rep[k] = "未同步"
            else:
                rep[k] = "已追平 (" + _fmt_logid(v) + ")"

        line = "-" * 44
        print(f"{'=' * 44}")
        print(f"  Raft 组 {group_id} 节点状态  id={node_id}  role={role}  running={running}")
        print(line)
        print(f"  当前 Term      : {metrics.get('current_term', '?')}")
        leader = _unwrap_some(metrics.get('current_leader', '?'))
        print(f"  当前 Leader    : {leader if leader is not None else '无'}")
        print(f"  投票           : {_fmt_vote(metrics.get('vote', '?'))}")
        last_idx = _unwrap_some(metrics.get('last_log_index', '?'))
        print(f"  最后日志索引   : {last_idx if last_idx is not None else '无'}")
        print(f"  最后应用       : {_fmt_logid(metrics.get('last_applied', '?'))}")
        print(f"  集群成员       : {members}")
        print(line)
        print(f"  成员地址:")
        if nodes_sorted:
            for nid, addr in nodes_sorted:
                print(f"    {nid} -> {addr.strip(chr(34))}")
        else:
            print("    (无)")
        print(f"  复制进度:")
        if rep:
            for nid, state in sorted(rep.items(), key=lambda x: int(x[0]) if x[0].isdigit() else x[0]):
                print(f"    {nid} -> {state}")
        else:
            print("    (无)")
        print(f"{'=' * 44}")
    except Exception:
        # 解析失败时回退到原始文本
        print(status)


def cmd_get_status(addr, group_id):
    import grpc

    pb2, svc = _stub(addr)
    if group_id is not None:
        # 指定组：只查询该组
        resp = svc.GetStatus(pb2.GetStatusRequest(group_id=group_id))
        _print_status(resp.status, group_id)
        return

    # 未指定组：探测本节点运行的数字组（组号递增；连续 3 个组不存在即停止）。
    # 注意：group_id 是字符串，这里只探测常见的数字命名组，自定义名称的组请显式 --group。
    n = 1
    miss = 0
    found_any = False
    while n <= 64:
        g = str(n)
        try:
            resp = svc.GetStatus(pb2.GetStatusRequest(group_id=g))
        except grpc.RpcError as e:
            if "不存在或未启动" in (e.details() or ""):
                miss += 1
                if miss >= 3:
                    break  # 连续 3 个组不存在，视为已到最大组号
                n += 1
                continue
            raise
        miss = 0
        found_any = True
        print(f"----- group-{g} -----")
        _print_status(resp.status, g)
        n += 1
    if not found_any:
        print("(本节点未运行任何 Raft 组)")


def cmd_list_peers(addr, group_id):
    pb2, svc = _stub(addr)
    resp = svc.ListPeers(pb2.ListPeersRequest(group_id=group_id))
    print(f"{'=' * 44}")
    print(f"  Raft 组 {group_id} 集群成员（共 {len(resp.peers)} 个节点）")
    print(f"{'-' * 44}")
    if not resp.peers:
        print("    (集群为空)")
        print(f"{'=' * 44}")
        return
    print(f"  {'ID':<6}{'ADDR'}")
    for p in resp.peers:
        print(f"  {p.id:<6}{p.addr}")
    print(f"{'=' * 44}")


def cmd_add_peer(addr, group_id, peer_id, peer_addr):
    pb2, svc = _stub(addr)
    req = pb2.AddPeerRequest(group_id=group_id, id=int(peer_id), addr=peer_addr)
    resp = svc.AddPeer(req)
    print(f"add-peer(组{group_id}) {peer_id} {peer_addr}: {'成功' if resp.success else '失败'} {resp.message}")


def cmd_remove_peer(addr, group_id, peer_id):
    pb2, svc = _stub(addr)
    resp = svc.RemovePeer(pb2.RemovePeerRequest(group_id=group_id, id=int(peer_id)))
    print(f"remove-peer(组{group_id}) {peer_id}: {'成功' if resp.success else '失败'} {resp.message}")


# ── data_key 编码（在客户端完成：table + 0x00 + key；Rust 端无 table 概念）──

def _data_key(table, key: bytes) -> bytes:
    """生成最终落盘键：table 字节 + NUL + key（table 为空时仅 NUL 前缀）"""
    t = table.encode("utf-8")
    return t + b"\x00" + key


def _split_data_key(k: bytes):
    """解析落盘键：返回 (table_bytes, raw_key)；无 NUL 时 table 为 None"""
    i = k.find(b"\x00")
    if i < 0:
        return None, k
    return k[:i], k[i + 1:]


def cmd_put(addr, group_id, table, key, value):
    pb2, svc = _stub(addr)
    resp = svc.Put(pb2.PutRequest(group_id=group_id, key=_data_key(table, _b(key)), value=_b(value)))
    print(f"put(组{group_id}): {'成功' if resp.success else '失败'} {resp.message}")


def cmd_get(addr, group_id, table, key):
    pb2, svc = _stub(addr)
    resp = svc.Get(pb2.GetRequest(group_id=group_id, key=_data_key(table, _b(key))))
    if resp.found:
        print(resp.value.decode("utf-8", errors="replace"))
    else:
        print("(未找到)")


def cmd_delete(addr, group_id, table, key):
    pb2, svc = _stub(addr)
    resp = svc.Delete(pb2.DeleteRequest(group_id=group_id, key=_data_key(table, _b(key))))
    print(f"delete(组{group_id}): {'成功' if resp.success else '失败'}")


def cmd_scan(addr, group_id, table=None):
    pb2, svc = _stub(addr)
    resp = svc.Scan(pb2.ScanRequest(group_id=group_id))
    if not resp.entries:
        print("(无数据)")
        return
    prefix = table.encode("utf-8") + b"\x00" if table is not None else None
    shown = 0
    for e in sorted(resp.entries, key=lambda x: x.key):
        if prefix is not None and not e.key.startswith(prefix):
            continue
        t, raw = _split_data_key(e.key)
        t_name = t.decode("utf-8", errors="replace") if t is not None else "<无前缀>"
        r = raw.decode("utf-8", errors="replace")
        v = e.value.decode("utf-8", errors="replace")
        print(f"[{t_name}] {r} = {v}")
        shown += 1
    if shown == 0:
        print("(无匹配数据)")


# ── 数据落盘一致性校验（调用容器内 ldb 工具对比各组各节点的业务数据 KV 库）──

# 容器内业务数据库路径（与 prod_raft.yaml 的 raft_service.data_path 对应）
BUSINESS_DB = "/app/laoflch_raft_data/business.db"
# 双写副本业务库（与 prod_raft.yaml 的 raft_service.dual_write.storage.db_path 对应）
BACKUP_DB = "/app/laoflch_raft_data/business_backup.db"


def _docker_containers(prefix="raft-node"):
    """列出当前运行的 raft 容器名"""
    r = subprocess.run(
        ["docker", "ps", "--filter", f"name={prefix}", "--format", "{{.Names}}"],
        capture_output=True, text=True, timeout=30,
    )
    if r.returncode != 0:
        return []
    return [l.strip() for l in r.stdout.splitlines() if l.strip()]


def _ldb_dump(node, group_id, db_path=BUSINESS_DB):
    """docker exec 在节点上用 ldb 导出 group_{g} CF 的全部键值对（hex 格式）"""
    r = subprocess.run(
        ["docker", "exec", node, "ldb",
         f"--db={db_path}",
         f"--column_family=group_{group_id}",
         "dump", "--hex"],
        capture_output=True, text=True, timeout=120,
    )
    if r.returncode != 0:
        return None  # 库/CF 不存在（未参与此组、未启用双写）或 ldb 不可用
    # 提取 "0x<key> ==> 0x<value>" 行并排序，做归一化
    lines = sorted(
        l.strip() for l in r.stdout.splitlines()
        if re.match(r"^0x[0-9a-fA-F]+ ==> 0x[0-9a-fA-F]+$", l.strip())
    )
    return lines


def _digest_verdict(results):
    """根据各节点 hash 集合返回一致性判定"""
    digests = {d for _, d in results.values()}
    if len(digests) == 1:
        return "一致 OK"
    if len(results) == 1:
        return "仅 1 个节点，无法对比"
    return "不一致（可能复制尚未完成或配置异常）"


def cmd_check_data(group_id, node_count=None):
    nodes = sorted(_docker_containers())
    if not nodes:
        print("错误：未检测到运行中的 raft-node 容器，请先部署集群（./deploy_raft_cluster.sh up）")
        return
    if node_count is None:
        node_count = len(nodes)

    groups = [group_id] if group_id is not None else [str(i) for i in range(1, node_count + 1)]
    print("=" * 44)
    print("  业务数据落盘一致性校验（主库 business.db + backup 副本）")
    print("=" * 44)
    for g in groups:
        print(f"----- group-{g} -----")
        main_results = {}
        backup_results = {}
        for i in range(1, node_count + 1):
            node = f"raft-node-{i}"
            if node not in nodes:
                print(f"  {node}: (容器未运行，跳过)")
                continue
            lines = _ldb_dump(node, g, BUSINESS_DB)
            if lines is None:
                print(f"  {node}: (不参与此组或业务库无该 CF)")
                continue
            digest = hashlib.sha256("\n".join(lines).encode()).hexdigest()
            main_results[node] = (len(lines), digest)
            blines = _ldb_dump(node, g, BACKUP_DB)
            if blines is None:
                print(f"  {node}: (未启用双写或 backup 库无该 CF)")
            else:
                bdigest = hashlib.sha256("\n".join(blines).encode()).hexdigest()
                backup_results[node] = (len(blines), bdigest)
        if not main_results:
            print("  (无节点参与此组)")
            continue
        print("  主库 (business.db):")
        for node, (cnt, d) in sorted(main_results.items()):
            print(f"    {node}: 条目数 {cnt}  hash {d[:16]}")
        print(f"    结果: {_digest_verdict(main_results)}")
        if backup_results:
            print("  backup 库 (business_backup.db):")
            for node, (cnt, d) in sorted(backup_results.items()):
                print(f"    {node}: 条目数 {cnt}  hash {d[:16]}")
            print(f"    结果: {_digest_verdict(backup_results)}")
            # 主库 vs backup：逐节点比对（双写副本应与主库一致）
            cross = [n for n in main_results
                     if n in backup_results and main_results[n][1] != backup_results[n][1]]
            print("    主库 vs backup: " + ("一致 OK" if not cross else f"不一致节点 {cross}"))
        else:
            print("  backup 库 (business_backup.db): (无节点启用双写，跳过对比)")
    print("=" * 44)


def _container_host_port(node):
    """通过 docker port 查询容器 29999/tcp 的宿主端口（如 raft-node-1 -> 29999）"""
    r = subprocess.run(
        ["docker", "port", node, "29999/tcp"],
        capture_output=True, text=True, timeout=15,
    )
    if r.returncode != 0 or not r.stdout.strip():
        return None
    # 输出形如 "0.0.0.0:29999"
    for line in r.stdout.splitlines():
        line = line.strip()
        if ":" in line:
            return line.rsplit(":", 1)[-1]
    return None


def cmd_status_all():
    """不带 --addr 时：遍历所有 raft-node 容器，逐个显示其上所有组的状态"""
    nodes = sorted(_docker_containers())
    if not nodes:
        print("未检测到运行中的 raft-node 容器，请先部署集群（./deploy_raft_cluster.sh up）")
        return
    for node in nodes:
        port = _container_host_port(node)
        if port is None:
            print(f"===== {node}: 无法获取宿主端口，跳过 =====")
            continue
        print(f"########## {node} (127.0.0.1:{port}) ##########")
        cmd_get_status(f"127.0.0.1:{port}", None)
        print()


def main():
    parser = argparse.ArgumentParser(description="laoflchdb Raft 集群管理客户端")
    parser.add_argument("--addr", default=None,
                        help="Raft 节点宿主地址，如 127.0.0.1:29999（status 不指定时遍历显示所有节点；其余命令默认 127.0.0.1:29999）")
    parser.add_argument("--group", type=str, default=None,
                        help="目标 Raft 组 ID（字符串，如 1 或 orders；不指定时 status 显示本节点所有数字组的状态，其余命令默认组 1）")
    parser.add_argument("--nodes", type=int, default=None,
                        help="集群节点数（check-data 用，默认自动检测 raft-node 容器数）")
    sub = parser.add_subparsers(dest="command", required=True)

    sub.add_parser("status", help="查看节点 Raft 状态")
    sub.add_parser("list-peers", help="查看集群成员")
    sub.add_parser("check-data", help="校验各组各节点业务数据落盘是否一致（主库 business.db + backup 副本，调用容器内 ldb）")
    p = sub.add_parser("add-peer", help="向集群添加成员")
    p.add_argument("id", type=int)
    p.add_argument("peer_addr", help="节点地址，如 raft-node-2:29999")
    p = sub.add_parser("remove-peer", help="从集群移除成员")
    p.add_argument("id", type=int)
    p = sub.add_parser("put", help="写入 KV")
    p.add_argument("table")
    p.add_argument("key")
    p.add_argument("value")
    p = sub.add_parser("get", help="读取 KV")
    p.add_argument("table")
    p.add_argument("key")
    p = sub.add_parser("delete", help="删除 KV")
    p.add_argument("table")
    p.add_argument("key")
    p = sub.add_parser("scan", help="扫描 KV（可 --table 过滤显示；落盘键由客户端编码）")
    p.add_argument("--table", default=None, help="按 table 过滤显示（可选）")

    args = parser.parse_args()
    cmd = args.command
    g = args.group  # None 时：status 探测所有组，其余命令默认组 1
    addr = args.addr if args.addr is not None else "127.0.0.1:29999"
    if cmd in ("status", "get-status"):  # get-status 为兼容旧用法
        if args.addr is None:
            cmd_status_all()
        else:
            cmd_get_status(args.addr, g)
    elif cmd == "check-data":
        cmd_check_data(g, args.nodes)
    elif cmd == "list-peers":
        cmd_list_peers(addr, g if g is not None else "1")
    elif cmd == "add-peer":
        cmd_add_peer(addr, g if g is not None else "1", args.id, args.peer_addr)
    elif cmd == "remove-peer":
        cmd_remove_peer(addr, g if g is not None else "1", args.id)
    elif cmd == "put":
        cmd_put(addr, g if g is not None else "1", args.table, args.key, args.value)
    elif cmd == "get":
        cmd_get(addr, g if g is not None else "1", args.table, args.key)
    elif cmd == "delete":
        cmd_delete(addr, g if g is not None else "1", args.table, args.key)
    elif cmd == "scan":
        cmd_scan(addr, g if g is not None else "1", args.table)
    else:
        parser.print_help()


if __name__ == "__main__":
    main()
