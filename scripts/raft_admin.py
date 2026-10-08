#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""laoflchdb Raft 集群管理客户端（宿主机使用）

通过 gRPC 访问 Raft 节点的状态查询与管理接口。
默认访问 127.0.0.1:29999（部署脚本中 node-1 的宿主映射端口）。

用法示例：
  python3 scripts/raft_admin.py --addr 127.0.0.1:29999 --group 1 get-status
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
import sys
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


def _print_status(status: str, group_id: int = 1):
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
    pb2, svc = _stub(addr)
    resp = svc.GetStatus(pb2.GetStatusRequest(group_id=group_id))
    _print_status(resp.status, group_id)


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


def cmd_put(addr, group_id, table, key, value):
    pb2, svc = _stub(addr)
    resp = svc.Put(pb2.PutRequest(group_id=group_id, table=table, key=_b(key), value=_b(value)))
    print(f"put(组{group_id}): {'成功' if resp.success else '失败'} {resp.message}")


def cmd_get(addr, group_id, table, key):
    pb2, svc = _stub(addr)
    resp = svc.Get(pb2.GetRequest(group_id=group_id, table=table, key=_b(key)))
    if resp.found:
        print(resp.value.decode("utf-8", errors="replace"))
    else:
        print("(未找到)")


def cmd_delete(addr, group_id, table, key):
    pb2, svc = _stub(addr)
    resp = svc.Delete(pb2.DeleteRequest(group_id=group_id, table=table, key=_b(key)))
    print(f"delete(组{group_id}): {'成功' if resp.success else '失败'}")


def cmd_scan(addr, group_id, table):
    pb2, svc = _stub(addr)
    resp = svc.Scan(pb2.ScanRequest(group_id=group_id, table=table))
    if not resp.entries:
        print("(表为空)")
        return
    for e in resp.entries:
        k = e.key.decode("utf-8", errors="replace")
        v = e.value.decode("utf-8", errors="replace")
        print(f"{k} = {v}")


def main():
    parser = argparse.ArgumentParser(description="laoflchdb Raft 集群管理客户端")
    parser.add_argument("--addr", default="127.0.0.1:29999",
                        help="Raft 节点宿主地址（默认 127.0.0.1:29999）")
    parser.add_argument("--group", type=int, default=1,
                        help="目标 Raft 组 ID（默认 1）")
    sub = parser.add_subparsers(dest="command", required=True)

    sub.add_parser("get-status", help="查看节点 Raft 状态")
    sub.add_parser("list-peers", help="查看集群成员")
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
    p = sub.add_parser("scan", help="扫描表")
    p.add_argument("table")

    args = parser.parse_args()
    cmd = args.command
    g = args.group
    if cmd == "get-status":
        cmd_get_status(args.addr, g)
    elif cmd == "list-peers":
        cmd_list_peers(args.addr, g)
    elif cmd == "add-peer":
        cmd_add_peer(args.addr, g, args.id, args.peer_addr)
    elif cmd == "remove-peer":
        cmd_remove_peer(args.addr, g, args.id)
    elif cmd == "put":
        cmd_put(args.addr, g, args.table, args.key, args.value)
    elif cmd == "get":
        cmd_get(args.addr, g, args.table, args.key)
    elif cmd == "delete":
        cmd_delete(args.addr, g, args.table, args.key)
    elif cmd == "scan":
        cmd_scan(args.addr, g, args.table)
    else:
        parser.print_help()


if __name__ == "__main__":
    main()
