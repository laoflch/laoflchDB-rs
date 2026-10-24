//! laoflchdb_graph_service
//!
//! 图服务：提供内置的属性图 GraphQL schema（查询 + 增删改），数据存储于 kv_rocksdb 引擎。
//!
//! - gRPC：`GraphService.Execute(GraphQlRequest)`（proto 中写为 `GraphQLRequest`，prost 生成名如此）
//! - REST：`POST /graphql`（由 [`create_rest_router`] 提供，挂载点由上层决定）
//!
//! # 存储布局
//!
//! 所有图数据存放在同一个 table（RocksDB 中对应一个 column family）内，
//! 键使用 NUL (`\0`) 作为分隔符，值为 JSON：
//!
//! - 节点：`N\0{node_type}\0{id}` → 属性 JSON 对象
//! - 边：`E\0{edge_type}\0{from_type}\0{from_id}\0{to_type}\0{to_id}` → 属性 JSON 对象
//! - 出边邻接：`O\0{from_type}\0{from_id}\0{edge_type}\0{to_type}\0{to_id}` → 空值
//! - 入边邻接：`I\0{to_type}\0{to_id}\0{edge_type}\0{from_type}\0{from_id}` → 空值
//!
//! 本服务的 mutation 会同时维护节点键、边键以及双向邻接键。
//! 键构造函数 [`node_key`] / [`edge_key`] / [`out_edge_key`] / [`in_edge_key`] 已公开，
//! 供外部写入方复用，保证布局一致。

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use async_graphql::{
    Context, EmptySubscription, Enum, Error, Json, Object, Result as GqlResult, Schema,
    SimpleObject,
};
use laoflchdb_engines::{EngineOptions, KvStorage};
use laoflchdb_kv_rocksdb_engine::KVRocksDBEngine;

pub mod proto {
    pub use laoflchdb_graph_service_proto::proto::*;
}
mod rest;
pub use rest::create_rest_router;

/// 键分隔符（NUL，正常业务字符串中不会出现）
const SEP: u8 = 0;

fn join_parts(parts: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(parts.iter().map(|p| p.len() + 1).sum());
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            out.push(SEP);
        }
        out.extend_from_slice(part);
    }
    out
}

fn utf8(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// 节点键：`N\0{node_type}\0{id}`
pub fn node_key(node_type: &str, id: &str) -> Vec<u8> {
    join_parts(&[b"N", node_type.as_bytes(), id.as_bytes()])
}

/// 边键：`E\0{edge_type}\0{from_type}\0{from_id}\0{to_type}\0{to_id}`
pub fn edge_key(
    edge_type: &str,
    from_type: &str,
    from_id: &str,
    to_type: &str,
    to_id: &str,
) -> Vec<u8> {
    join_parts(&[
        b"E",
        edge_type.as_bytes(),
        from_type.as_bytes(),
        from_id.as_bytes(),
        to_type.as_bytes(),
        to_id.as_bytes(),
    ])
}

/// 出边邻接键：`O\0{from_type}\0{from_id}\0{edge_type}\0{to_type}\0{to_id}`
pub fn out_edge_key(
    from_type: &str,
    from_id: &str,
    edge_type: &str,
    to_type: &str,
    to_id: &str,
) -> Vec<u8> {
    join_parts(&[
        b"O",
        from_type.as_bytes(),
        from_id.as_bytes(),
        edge_type.as_bytes(),
        to_type.as_bytes(),
        to_id.as_bytes(),
    ])
}

/// 入边邻接键：`I\0{to_type}\0{to_id}\0{edge_type}\0{from_type}\0{from_id}`
pub fn in_edge_key(
    to_type: &str,
    to_id: &str,
    edge_type: &str,
    from_type: &str,
    from_id: &str,
) -> Vec<u8> {
    join_parts(&[
        b"I",
        to_type.as_bytes(),
        to_id.as_bytes(),
        edge_type.as_bytes(),
        from_type.as_bytes(),
        from_id.as_bytes(),
    ])
}

fn node_prefix(node_type: &str) -> Vec<u8> {
    join_parts(&[b"N", node_type.as_bytes(), b""])
}

/// 图服务配置
#[derive(Debug, Clone)]
pub struct GraphServiceConfig {
    /// kv_rocksdb 数据目录
    pub db_path: String,
    /// 引擎 schema 名称（仅用于引擎初始化）
    pub schema_name: String,
    /// 图数据所在 table（RocksDB column family）
    pub table_name: String,
    /// 单次查询返回的最大条数（0 表示不限制）
    pub max_page_size: usize,
}

impl Default for GraphServiceConfig {
    fn default() -> Self {
        Self {
            db_path: "./laoflch_graph_data".to_string(),
            schema_name: "graph".to_string(),
            table_name: "graph".to_string(),
            max_page_size: 1000,
        }
    }
}

/// GraphQL 执行上下文数据
struct GraphData {
    storage: Arc<dyn KvStorage>,
    table: String,
    max_page_size: usize,
}

/// 邻接边引用：(edge_type, from_type, from_id, to_type, to_id)
type EdgeRef = (String, String, String, String, String);

#[derive(SimpleObject, Clone)]
pub struct GqlNode {
    /// 节点 ID
    pub id: String,
    /// 节点类型
    pub node_type: String,
    /// 节点属性（JSON 对象）
    pub properties: Json<serde_json::Value>,
}

#[derive(SimpleObject, Clone)]
pub struct GqlEdge {
    /// 边类型
    pub edge_type: String,
    pub from_type: String,
    pub from_id: String,
    pub to_type: String,
    pub to_id: String,
    /// 边属性（JSON 对象）
    pub properties: Json<serde_json::Value>,
}

#[derive(SimpleObject, Clone)]
pub struct GqlNeighbor {
    pub node: GqlNode,
    pub edge: GqlEdge,
}

#[derive(SimpleObject, Clone)]
pub struct GqlPath {
    pub nodes: Vec<GqlNode>,
    pub edges: Vec<GqlEdge>,
    /// 路径长度（边数）
    pub length: i32,
}

#[derive(SimpleObject, Clone)]
pub struct GqlStats {
    pub node_count: i64,
    pub edge_count: i64,
}

/// 遍历方向
#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug, Default)]
pub enum Direction {
    /// 沿出边（默认）
    #[default]
    Out,
    /// 沿入边
    In,
    /// 双向
    Both,
}

fn parse_props(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice(bytes).unwrap_or_else(|e| {
        log::warn!("图数据属性 JSON 解析失败: {}", e);
        serde_json::Value::Object(serde_json::Map::new())
    })
}

fn page(data: &GraphData, limit: Option<i32>, offset: Option<i32>) -> (usize, usize) {
    let limit = limit.unwrap_or(50).max(0) as usize;
    let limit = if data.max_page_size == 0 {
        limit
    } else {
        limit.min(data.max_page_size)
    };
    let offset = offset.unwrap_or(0).max(0) as usize;
    (limit, offset)
}

async fn load_node(data: &GraphData, node_type: &str, id: &str) -> GqlResult<Option<GqlNode>> {
    let value = data
        .storage
        .get(&data.table, &node_key(node_type, id))
        .await
        .map_err(|e| Error::new(e.to_string()))?;
    Ok(value.map(|bytes| GqlNode {
        id: id.to_string(),
        node_type: node_type.to_string(),
        properties: Json(parse_props(&bytes)),
    }))
}

async fn get_edge(
    data: &GraphData,
    edge_type: &str,
    from_type: &str,
    from_id: &str,
    to_type: &str,
    to_id: &str,
) -> GqlResult<Option<GqlEdge>> {
    let value = data
        .storage
        .get(
            &data.table,
            &edge_key(edge_type, from_type, from_id, to_type, to_id),
        )
        .await
        .map_err(|e| Error::new(e.to_string()))?;
    Ok(value.map(|b| GqlEdge {
        edge_type: edge_type.to_string(),
        from_type: from_type.to_string(),
        from_id: from_id.to_string(),
        to_type: to_type.to_string(),
        to_id: to_id.to_string(),
        properties: Json(parse_props(&b)),
    }))
}

async fn load_edge(
    data: &GraphData,
    edge_type: &str,
    from_type: &str,
    from_id: &str,
    to_type: &str,
    to_id: &str,
) -> GqlResult<GqlEdge> {
    Ok(get_edge(data, edge_type, from_type, from_id, to_type, to_id)
        .await?
        .unwrap_or_else(|| GqlEdge {
            edge_type: edge_type.to_string(),
            from_type: from_type.to_string(),
            from_id: from_id.to_string(),
            to_type: to_type.to_string(),
            to_id: to_id.to_string(),
            properties: Json(serde_json::Value::Object(serde_json::Map::new())),
        }))
}

/// 属性合并：旧值为对象时按键浅合并，否则直接替换
fn merge_props(existing: &[u8], patch: serde_json::Value) -> serde_json::Value {
    match (parse_props(existing), patch) {
        (serde_json::Value::Object(mut map), serde_json::Value::Object(extra)) => {
            for (k, v) in extra {
                map.insert(k, v);
            }
            serde_json::Value::Object(map)
        }
        (_, patch) => patch,
    }
}

/// 删除一条边的边键与双向邻接键
async fn delete_edge_keys(
    data: &GraphData,
    edge_type: &str,
    from_type: &str,
    from_id: &str,
    to_type: &str,
    to_id: &str,
) -> GqlResult<()> {
    let keys = [
        edge_key(edge_type, from_type, from_id, to_type, to_id),
        out_edge_key(from_type, from_id, edge_type, to_type, to_id),
        in_edge_key(to_type, to_id, edge_type, from_type, from_id),
    ];
    for key in keys {
        data.storage
            .delete(&data.table, &key)
            .await
            .map_err(|e| Error::new(e.to_string()))?;
    }
    Ok(())
}

/// 扫描指定节点的出边邻接键
async fn out_edges(
    data: &GraphData,
    from_type: &str,
    from_id: &str,
    edge_type: Option<&str>,
) -> GqlResult<Vec<EdgeRef>> {
    let (prefix, filtered) = match edge_type {
        Some(et) => (
            join_parts(&[
                b"O",
                from_type.as_bytes(),
                from_id.as_bytes(),
                et.as_bytes(),
                b"",
            ]),
            true,
        ),
        None => (
            join_parts(&[b"O", from_type.as_bytes(), from_id.as_bytes(), b""]),
            false,
        ),
    };

    let mut out = Vec::new();
    for (key, _) in data
        .storage
        .scan(&data.table)
        .await
        .map_err(|e| Error::new(e.to_string()))?
    {
        if !key.starts_with(&prefix) {
            continue;
        }
        let parts: Vec<&[u8]> = key[prefix.len()..].split(|b| *b == SEP).collect();
        let (et, to_type, to_id) = if filtered {
            if parts.len() != 2 {
                continue;
            }
            (edge_type.unwrap().to_string(), utf8(parts[0]), utf8(parts[1]))
        } else {
            if parts.len() != 3 {
                continue;
            }
            (utf8(parts[0]), utf8(parts[1]), utf8(parts[2]))
        };
        out.push((
            et,
            from_type.to_string(),
            from_id.to_string(),
            to_type,
            to_id,
        ));
    }
    Ok(out)
}

/// 扫描指定节点的入边邻接键
async fn in_edges(
    data: &GraphData,
    to_type: &str,
    to_id: &str,
    edge_type: Option<&str>,
) -> GqlResult<Vec<EdgeRef>> {
    let (prefix, filtered) = match edge_type {
        Some(et) => (
            join_parts(&[
                b"I",
                to_type.as_bytes(),
                to_id.as_bytes(),
                et.as_bytes(),
                b"",
            ]),
            true,
        ),
        None => (
            join_parts(&[b"I", to_type.as_bytes(), to_id.as_bytes(), b""]),
            false,
        ),
    };

    let mut out = Vec::new();
    for (key, _) in data
        .storage
        .scan(&data.table)
        .await
        .map_err(|e| Error::new(e.to_string()))?
    {
        if !key.starts_with(&prefix) {
            continue;
        }
        let parts: Vec<&[u8]> = key[prefix.len()..].split(|b| *b == SEP).collect();
        let (et, from_type, from_id) = if filtered {
            if parts.len() != 2 {
                continue;
            }
            (edge_type.unwrap().to_string(), utf8(parts[0]), utf8(parts[1]))
        } else {
            if parts.len() != 3 {
                continue;
            }
            (utf8(parts[0]), utf8(parts[1]), utf8(parts[2]))
        };
        out.push((
            et,
            from_type,
            from_id,
            to_type.to_string(),
            to_id.to_string(),
        ));
    }
    Ok(out)
}

/// GraphQL 查询根
pub struct QueryRoot;

#[Object]
impl QueryRoot {
    /// 按类型与 ID 获取单个节点
    async fn node(
        &self,
        ctx: &Context<'_>,
        node_type: String,
        id: String,
    ) -> GqlResult<Option<GqlNode>> {
        let data = ctx.data::<GraphData>()?;
        load_node(data, &node_type, &id).await
    }

    /// 分页列出指定类型的节点（按 ID 升序）
    async fn nodes(
        &self,
        ctx: &Context<'_>,
        node_type: String,
        limit: Option<i32>,
        offset: Option<i32>,
    ) -> GqlResult<Vec<GqlNode>> {
        let data = ctx.data::<GraphData>()?;
        let (limit, offset) = page(data, limit, offset);
        let prefix = node_prefix(&node_type);

        let mut nodes = Vec::new();
        for (key, value) in data
            .storage
            .scan(&data.table)
            .await
            .map_err(|e| Error::new(e.to_string()))?
        {
            if !key.starts_with(&prefix) {
                continue;
            }
            nodes.push(GqlNode {
                id: utf8(&key[prefix.len()..]),
                node_type: node_type.clone(),
                properties: Json(parse_props(&value)),
            });
        }
        nodes.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(nodes.into_iter().skip(offset).take(limit).collect())
    }

    /// 查询相邻节点（按指定方向与边类型过滤）
    async fn neighbors(
        &self,
        ctx: &Context<'_>,
        node_type: String,
        id: String,
        edge_type: Option<String>,
        #[graphql(default)] direction: Direction,
        limit: Option<i32>,
        offset: Option<i32>,
    ) -> GqlResult<Vec<GqlNeighbor>> {
        let data = ctx.data::<GraphData>()?;
        let (limit, offset) = page(data, limit, offset);

        // (边引用, 邻居节点类型, 邻居节点 ID)：出边的邻居是 to 端，入边的邻居是 from 端
        let mut refs: Vec<(EdgeRef, String, String)> = Vec::new();
        if matches!(direction, Direction::Out | Direction::Both) {
            for r in out_edges(data, &node_type, &id, edge_type.as_deref()).await? {
                refs.push((r.clone(), r.3, r.4));
            }
        }
        if matches!(direction, Direction::In | Direction::Both) {
            for r in in_edges(data, &node_type, &id, edge_type.as_deref()).await? {
                refs.push((r.clone(), r.1, r.2));
            }
        }

        // 去重并排序，保证结果稳定
        let mut seen = HashSet::new();
        refs.retain(|(r, _, _)| seen.insert(r.clone()));
        refs.sort_by(|a, b| a.0.cmp(&b.0));

        let mut out = Vec::new();
        for ((et, ft, fi, tt, ti), neighbor_type, neighbor_id) in
            refs.into_iter().skip(offset).take(limit)
        {
            let Some(node) = load_node(data, &neighbor_type, &neighbor_id).await? else {
                continue;
            };
            let edge = load_edge(data, &et, &ft, &fi, &tt, &ti).await?;
            out.push(GqlNeighbor { node, edge });
        }
        Ok(out)
    }

    /// 沿出边进行 BFS 最短路径查询（返回节点与边序列）
    async fn shortest_path(
        &self,
        ctx: &Context<'_>,
        from_type: String,
        from_id: String,
        to_type: String,
        to_id: String,
        edge_type: Option<String>,
        max_depth: Option<i32>,
    ) -> GqlResult<Option<GqlPath>> {
        let data = ctx.data::<GraphData>()?;
        let max_depth = max_depth.unwrap_or(6).clamp(1, 16) as usize;
        let start = (from_type, from_id);
        let goal = (to_type, to_id);

        if start == goal {
            let Some(node) = load_node(data, &start.0, &start.1).await? else {
                return Ok(None);
            };
            return Ok(Some(GqlPath {
                nodes: vec![node],
                edges: vec![],
                length: 0,
            }));
        }

        let mut visited: HashSet<(String, String)> = HashSet::new();
        visited.insert(start.clone());
        let mut parent: HashMap<(String, String), ((String, String), GqlEdge)> = HashMap::new();
        let mut queue: VecDeque<((String, String), usize)> = VecDeque::new();
        queue.push_back((start.clone(), 0));

        let mut found = false;
        while let Some((current, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }
            for (et, ft, fi, tt, ti) in
                out_edges(data, &current.0, &current.1, edge_type.as_deref()).await?
            {
                let next = (tt.clone(), ti.clone());
                if !visited.insert(next.clone()) {
                    continue;
                }
                let edge = load_edge(data, &et, &ft, &fi, &tt, &ti).await?;
                parent.insert(next.clone(), (current.clone(), edge));
                if next == goal {
                    found = true;
                    break;
                }
                queue.push_back((next, depth + 1));
            }
            if found {
                break;
            }
        }

        if !found {
            return Ok(None);
        }

        // 从 goal 回溯到 start
        let mut chain: Vec<((String, String), Option<GqlEdge>)> = Vec::new();
        let mut current = goal;
        loop {
            if current == start {
                chain.push((current.clone(), None));
                break;
            }
            let Some((prev, edge)) = parent.get(&current).cloned() else {
                return Ok(None);
            };
            chain.push((current.clone(), Some(edge)));
            current = prev;
        }
        chain.reverse();

        let mut nodes = Vec::with_capacity(chain.len());
        for ((node_type, id), _) in &chain {
            let Some(node) = load_node(data, node_type, id).await? else {
                // 路径上有节点缺失，视为不存在完整路径
                return Ok(None);
            };
            nodes.push(node);
        }
        let edges: Vec<GqlEdge> = chain
            .iter()
            .filter_map(|(_, edge)| edge.clone())
            .collect();
        let length = edges.len() as i32;
        Ok(Some(GqlPath {
            nodes,
            edges,
            length,
        }))
    }

    /// 图数据统计
    async fn stats(&self, ctx: &Context<'_>) -> GqlResult<GqlStats> {
        let data = ctx.data::<GraphData>()?;
        let all = data
            .storage
            .scan(&data.table)
            .await
            .map_err(|e| Error::new(e.to_string()))?;
        let node_count = all.iter().filter(|(k, _)| k.starts_with(b"N\0")).count() as i64;
        let edge_count = all.iter().filter(|(k, _)| k.starts_with(b"E\0")).count() as i64;
        Ok(GqlStats {
            node_count,
            edge_count,
        })
    }
}

/// 以 JSON 形式写入一条记录
async fn put_json(data: &GraphData, key: &[u8], value: &serde_json::Value) -> GqlResult<()> {
    let bytes = serde_json::to_vec(value).map_err(|e| Error::new(e.to_string()))?;
    data.storage
        .put(&data.table, key, &bytes)
        .await
        .map_err(|e| Error::new(e.to_string()))
}

/// GraphQL 变更根：节点与边的增删改
pub struct MutationRoot;

#[Object]
impl MutationRoot {
    /// 插入节点（节点已存在时报错）
    async fn insert_node(
        &self,
        ctx: &Context<'_>,
        node_type: String,
        id: String,
        properties: Option<Json<serde_json::Value>>,
    ) -> GqlResult<GqlNode> {
        let data = ctx.data::<GraphData>()?;
        if load_node(data, &node_type, &id).await?.is_some() {
            return Err(Error::new(format!("节点已存在: {}/{}", node_type, id)));
        }
        let props = properties
            .map(|j| j.0)
            .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
        put_json(data, &node_key(&node_type, &id), &props).await?;
        Ok(GqlNode {
            id,
            node_type,
            properties: Json(props),
        })
    }

    /// 更新节点属性（浅合并；节点不存在时报错）
    async fn update_node(
        &self,
        ctx: &Context<'_>,
        node_type: String,
        id: String,
        properties: Json<serde_json::Value>,
    ) -> GqlResult<GqlNode> {
        let data = ctx.data::<GraphData>()?;
        let existing = data
            .storage
            .get(&data.table, &node_key(&node_type, &id))
            .await
            .map_err(|e| Error::new(e.to_string()))?
            .ok_or_else(|| Error::new(format!("节点不存在: {}/{}", node_type, id)))?;
        let merged = merge_props(&existing, properties.0);
        put_json(data, &node_key(&node_type, &id), &merged).await?;
        Ok(GqlNode {
            id,
            node_type,
            properties: Json(merged),
        })
    }

    /// 删除节点及其关联的所有边
    async fn delete_node(
        &self,
        ctx: &Context<'_>,
        node_type: String,
        id: String,
    ) -> GqlResult<bool> {
        let data = ctx.data::<GraphData>()?;
        let existed = load_node(data, &node_type, &id).await?.is_some();

        let mut refs: Vec<EdgeRef> = Vec::new();
        refs.extend(out_edges(data, &node_type, &id, None).await?);
        refs.extend(in_edges(data, &node_type, &id, None).await?);
        for (et, ft, fi, tt, ti) in refs {
            delete_edge_keys(data, &et, &ft, &fi, &tt, &ti).await?;
        }

        data.storage
            .delete(&data.table, &node_key(&node_type, &id))
            .await
            .map_err(|e| Error::new(e.to_string()))?;
        Ok(existed)
    }

    /// 插入边（两端节点必须存在；边已存在时报错）
    #[allow(clippy::too_many_arguments)]
    async fn insert_edge(
        &self,
        ctx: &Context<'_>,
        edge_type: String,
        from_type: String,
        from_id: String,
        to_type: String,
        to_id: String,
        properties: Option<Json<serde_json::Value>>,
    ) -> GqlResult<GqlEdge> {
        let data = ctx.data::<GraphData>()?;
        if load_node(data, &from_type, &from_id).await?.is_none() {
            return Err(Error::new(format!(
                "起点节点不存在: {}/{}",
                from_type, from_id
            )));
        }
        if load_node(data, &to_type, &to_id).await?.is_none() {
            return Err(Error::new(format!(
                "终点节点不存在: {}/{}",
                to_type, to_id
            )));
        }
        if get_edge(data, &edge_type, &from_type, &from_id, &to_type, &to_id)
            .await?
            .is_some()
        {
            return Err(Error::new("边已存在"));
        }

        let props = properties
            .map(|j| j.0)
            .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
        put_json(
            data,
            &edge_key(&edge_type, &from_type, &from_id, &to_type, &to_id),
            &props,
        )
        .await?;
        // 维护双向邻接（值为空，仅作为索引）
        data.storage
            .put(
                &data.table,
                &out_edge_key(&from_type, &from_id, &edge_type, &to_type, &to_id),
                &[],
            )
            .await
            .map_err(|e| Error::new(e.to_string()))?;
        data.storage
            .put(
                &data.table,
                &in_edge_key(&to_type, &to_id, &edge_type, &from_type, &from_id),
                &[],
            )
            .await
            .map_err(|e| Error::new(e.to_string()))?;

        Ok(GqlEdge {
            edge_type,
            from_type,
            from_id,
            to_type,
            to_id,
            properties: Json(props),
        })
    }

    /// 更新边属性（浅合并；边不存在时报错）
    #[allow(clippy::too_many_arguments)]
    async fn update_edge(
        &self,
        ctx: &Context<'_>,
        edge_type: String,
        from_type: String,
        from_id: String,
        to_type: String,
        to_id: String,
        properties: Json<serde_json::Value>,
    ) -> GqlResult<GqlEdge> {
        let data = ctx.data::<GraphData>()?;
        let key = edge_key(&edge_type, &from_type, &from_id, &to_type, &to_id);
        let existing = data
            .storage
            .get(&data.table, &key)
            .await
            .map_err(|e| Error::new(e.to_string()))?
            .ok_or_else(|| Error::new("边不存在"))?;
        let merged = merge_props(&existing, properties.0);
        put_json(data, &key, &merged).await?;
        Ok(GqlEdge {
            edge_type,
            from_type,
            from_id,
            to_type,
            to_id,
            properties: Json(merged),
        })
    }

    /// 删除边（同时删除双向邻接键）
    #[allow(clippy::too_many_arguments)]
    async fn delete_edge(
        &self,
        ctx: &Context<'_>,
        edge_type: String,
        from_type: String,
        from_id: String,
        to_type: String,
        to_id: String,
    ) -> GqlResult<bool> {
        let data = ctx.data::<GraphData>()?;
        let existed = get_edge(data, &edge_type, &from_type, &from_id, &to_type, &to_id)
            .await?
            .is_some();
        delete_edge_keys(data, &edge_type, &from_type, &from_id, &to_type, &to_id).await?;
        Ok(existed)
    }
}

/// GraphQL 执行结果
#[derive(Debug, Clone)]
pub struct GraphQLOutcome {
    pub data: serde_json::Value,
    pub errors: Vec<serde_json::Value>,
}

impl GraphQLOutcome {
    pub fn is_success(&self) -> bool {
        self.errors.is_empty()
    }
}

/// 图服务实现
pub struct GraphServiceImpl {
    schema: Schema<QueryRoot, MutationRoot, EmptySubscription>,
}

impl GraphServiceImpl {
    /// 基于 kv_rocksdb 创建图服务
    pub async fn new(
        config: &GraphServiceConfig,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let engine = KVRocksDBEngine::new(&EngineOptions {
            db_path: config.db_path.clone(),
            schema_name: config.schema_name.clone(),
        })?;
        engine.ensure_table(&config.table_name).await?;
        let storage: Arc<dyn KvStorage> = Arc::new(engine);
        Ok(Self::from_storage(
            storage,
            config.table_name.clone(),
            config.max_page_size,
        ))
    }

    /// 基于任意 KvStorage 创建图服务（便于测试与复用）
    pub fn from_storage(
        storage: Arc<dyn KvStorage>,
        table_name: String,
        max_page_size: usize,
    ) -> Self {
        let schema = Schema::build(QueryRoot, MutationRoot, EmptySubscription)
            .data(GraphData {
                storage,
                table: table_name,
                max_page_size,
            })
            .finish();
        Self { schema }
    }

    /// 执行 GraphQL 查询（gRPC 与 REST 共用）
    pub async fn execute_graphql(
        &self,
        query: &str,
        variables: Option<serde_json::Value>,
        operation_name: Option<String>,
    ) -> GraphQLOutcome {
        let mut request = async_graphql::Request::new(query);
        if let Some(variables) = variables {
            request = request.variables(async_graphql::Variables::from_json(variables));
        }
        if let Some(operation_name) = operation_name {
            if !operation_name.is_empty() {
                request = request.operation_name(operation_name);
            }
        }

        let response = self.schema.execute(request).await;
        let data = serde_json::to_value(&response.data)
            .unwrap_or(serde_json::Value::Null);
        let errors = response
            .errors
            .iter()
            .filter_map(|e| serde_json::to_value(e).ok())
            .collect();
        GraphQLOutcome { data, errors }
    }
}

#[tonic::async_trait]
impl proto::graph_service_server::GraphService for GraphServiceImpl {
    async fn execute(
        &self,
        request: tonic::Request<proto::GraphQlRequest>,
    ) -> Result<tonic::Response<proto::GraphQlResponse>, tonic::Status> {
        let req = request.into_inner();

        let variables = if req.variables_json.trim().is_empty() {
            None
        } else {
            Some(
                serde_json::from_str::<serde_json::Value>(&req.variables_json).map_err(|e| {
                    tonic::Status::invalid_argument(format!("variables_json 解析失败: {}", e))
                })?,
            )
        };

        let operation_name = if req.operation_name.trim().is_empty() {
            None
        } else {
            Some(req.operation_name)
        };

        let outcome = self
            .execute_graphql(&req.query, variables, operation_name)
            .await;

        Ok(tonic::Response::new(proto::GraphQlResponse {
            success: outcome.is_success(),
            data_json: serde_json::to_string(&outcome.data)
                .unwrap_or_else(|_| "null".to_string()),
            errors_json: serde_json::to_string(&outcome.errors)
                .unwrap_or_else(|_| "[]".to_string()),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemStorage {
        tables: Mutex<HashMap<String, HashMap<Vec<u8>, Vec<u8>>>>,
    }

    #[async_trait::async_trait]
    impl KvStorage for MemStorage {
        fn engine_type(&self) -> &str {
            "memory"
        }

        async fn ensure_table(
            &self,
            table: &str,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            self.tables
                .lock()
                .unwrap()
                .entry(table.to_string())
                .or_default();
            Ok(())
        }

        async fn put(
            &self,
            table: &str,
            key: &[u8],
            value: &[u8],
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            self.tables
                .lock()
                .unwrap()
                .entry(table.to_string())
                .or_default()
                .insert(key.to_vec(), value.to_vec());
            Ok(())
        }

        async fn get(
            &self,
            table: &str,
            key: &[u8],
        ) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self
                .tables
                .lock()
                .unwrap()
                .get(table)
                .and_then(|t| t.get(key).cloned()))
        }

        async fn delete(
            &self,
            table: &str,
            key: &[u8],
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            self.tables
                .lock()
                .unwrap()
                .entry(table.to_string())
                .or_default()
                .remove(key);
            Ok(())
        }

        async fn scan(
            &self,
            table: &str,
        ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self
                .tables
                .lock()
                .unwrap()
                .get(table)
                .map(|t| t.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                .unwrap_or_default())
        }

        async fn clear(
            &self,
            table: &str,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            self.tables
                .lock()
                .unwrap()
                .entry(table.to_string())
                .or_default()
                .clear();
            Ok(())
        }
    }

    async fn put_node(
        storage: &MemStorage,
        node_type: &str,
        id: &str,
        props: serde_json::Value,
    ) {
        storage
            .put(
                "graph",
                &node_key(node_type, id),
                &serde_json::to_vec(&props).unwrap(),
            )
            .await
            .unwrap();
    }

    async fn put_edge(
        storage: &MemStorage,
        edge_type: &str,
        from_type: &str,
        from_id: &str,
        to_type: &str,
        to_id: &str,
        props: serde_json::Value,
    ) {
        storage
            .put(
                "graph",
                &edge_key(edge_type, from_type, from_id, to_type, to_id),
                &serde_json::to_vec(&props).unwrap(),
            )
            .await
            .unwrap();
        storage
            .put(
                "graph",
                &out_edge_key(from_type, from_id, edge_type, to_type, to_id),
                b"",
            )
            .await
            .unwrap();
        storage
            .put(
                "graph",
                &in_edge_key(to_type, to_id, edge_type, from_type, from_id),
                b"",
            )
            .await
            .unwrap();
    }

    async fn seed(storage: &MemStorage) {
        put_node(storage, "person", "p1", serde_json::json!({"name": "Alice"}))
            .await;
        put_node(storage, "person", "p2", serde_json::json!({"name": "Bob"}))
            .await;
        put_node(storage, "person", "p3", serde_json::json!({"name": "Carol"}))
            .await;
        put_node(storage, "company", "c1", serde_json::json!({"name": "Acme"}))
            .await;

        put_edge(
            storage,
            "knows",
            "person",
            "p1",
            "person",
            "p2",
            serde_json::json!({"since": 2020}),
        )
        .await;
        put_edge(
            storage,
            "knows",
            "person",
            "p2",
            "person",
            "p3",
            serde_json::json!({"since": 2021}),
        )
        .await;
        put_edge(
            storage,
            "works_at",
            "person",
            "p1",
            "company",
            "c1",
            serde_json::json!({"role": "engineer"}),
        )
        .await;
    }

    fn service(storage: Arc<MemStorage>) -> GraphServiceImpl {
        GraphServiceImpl::from_storage(storage, "graph".to_string(), 100)
    }

    async fn run(svc: &GraphServiceImpl, query: &str) -> serde_json::Value {
        let outcome = svc.execute_graphql(query, None, None).await;
        assert!(outcome.is_success(), "GraphQL errors: {:?}", outcome.errors);
        outcome.data
    }

    #[tokio::test]
    async fn query_node() {
        let storage = Arc::new(MemStorage::default());
        seed(&storage).await;
        let svc = service(storage);

        let data = run(
            &svc,
            r#"{ node(nodeType: "person", id: "p1") { id nodeType properties } }"#,
        )
        .await;
        assert_eq!(data["node"]["id"], "p1");
        assert_eq!(data["node"]["nodeType"], "person");
        assert_eq!(data["node"]["properties"]["name"], "Alice");

        let missing = run(
            &svc,
            r#"{ node(nodeType: "person", id: "nope") { id } }"#,
        )
        .await;
        assert!(missing["node"].is_null());
    }

    #[tokio::test]
    async fn query_nodes_with_pagination() {
        let storage = Arc::new(MemStorage::default());
        seed(&storage).await;
        let svc = service(storage);

        let data = run(
            &svc,
            r#"{ nodes(nodeType: "person", limit: 1, offset: 1) { id } }"#,
        )
        .await;
        assert_eq!(data["nodes"].as_array().unwrap().len(), 1);
        assert_eq!(data["nodes"][0]["id"], "p2");
    }

    #[tokio::test]
    async fn query_neighbors() {
        let storage = Arc::new(MemStorage::default());
        seed(&storage).await;
        let svc = service(storage);

        let data = run(
            &svc,
            r#"{ neighbors(nodeType: "person", id: "p1") { node { id } edge { edgeType fromId toId } } }"#,
        )
        .await;
        let arr = data["neighbors"].as_array().unwrap();
        assert_eq!(arr.len(), 2);

        let in_data = run(
            &svc,
            r#"{ neighbors(nodeType: "person", id: "p3", direction: IN) { node { id } edge { edgeType } } }"#,
        )
        .await;
        assert_eq!(in_data["neighbors"][0]["node"]["id"], "p2");
        assert_eq!(in_data["neighbors"][0]["edge"]["edgeType"], "knows");

        let filtered = run(
            &svc,
            r#"{ neighbors(nodeType: "person", id: "p1", edgeType: "works_at") { node { id } } }"#,
        )
        .await;
        assert_eq!(filtered["neighbors"].as_array().unwrap().len(), 1);
        assert_eq!(filtered["neighbors"][0]["node"]["id"], "c1");
    }

    #[tokio::test]
    async fn query_shortest_path() {
        let storage = Arc::new(MemStorage::default());
        seed(&storage).await;
        let svc = service(storage);

        let data = run(
            &svc,
            r#"{ shortestPath(fromType: "person", fromId: "p1", toType: "person", toId: "p3") {
                    length nodes { id } edges { edgeType }
               } }"#,
        )
        .await;
        assert_eq!(data["shortestPath"]["length"], 2);
        let ids: Vec<&str> = data["shortestPath"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["p1", "p2", "p3"]);

        // 无路径
        let none = run(
            &svc,
            r#"{ shortestPath(fromType: "person", fromId: "p3", toType: "person", toId: "p1") { length } }"#,
        )
        .await;
        assert!(none["shortestPath"].is_null());
    }

    #[tokio::test]
    async fn query_stats() {
        let storage = Arc::new(MemStorage::default());
        seed(&storage).await;
        let svc = service(storage);

        let data = run(&svc, r#"{ stats { nodeCount edgeCount } }"#).await;
        assert_eq!(data["stats"]["nodeCount"], 4);
        assert_eq!(data["stats"]["edgeCount"], 3);
    }

    #[tokio::test]
    async fn mutation_node_crud() {
        let storage = Arc::new(MemStorage::default());
        seed(&storage).await;
        let svc = service(storage);

        // 插入
        let data = run(
            &svc,
            r#"mutation { insertNode(nodeType: "person", id: "p4", properties: {name: "Dave"}) { id properties } }"#,
        )
        .await;
        assert_eq!(data["insertNode"]["properties"]["name"], "Dave");

        // 重复插入报错
        let dup = svc
            .execute_graphql(
                r#"mutation { insertNode(nodeType: "person", id: "p4") { id } }"#,
                None,
                None,
            )
            .await;
        assert!(!dup.is_success());

        // 更新（浅合并，保留已有字段）
        let data = run(
            &svc,
            r#"mutation { updateNode(nodeType: "person", id: "p4", properties: {age: 30}) { properties } }"#,
        )
        .await;
        assert_eq!(data["updateNode"]["properties"]["name"], "Dave");
        assert_eq!(data["updateNode"]["properties"]["age"], 30);

        // 删除
        let data = run(
            &svc,
            r#"mutation { deleteNode(nodeType: "person", id: "p4") }"#,
        )
        .await;
        assert_eq!(data["deleteNode"], true);

        let gone = run(&svc, r#"{ node(nodeType: "person", id: "p4") { id } }"#).await;
        assert!(gone["node"].is_null());
    }

    #[tokio::test]
    async fn mutation_edge_crud_and_cascade_delete() {
        let storage = Arc::new(MemStorage::default());
        seed(&storage).await;
        let svc = service(storage);

        // 插入边：p1 -> p3
        let data = run(
            &svc,
            r#"mutation { insertEdge(edgeType: "knows", fromType: "person", fromId: "p1", toType: "person", toId: "p3", properties: {since: 2024}) { edgeType properties } }"#,
        )
        .await;
        assert_eq!(data["insertEdge"]["properties"]["since"], 2024);

        // 两端节点必须存在
        let bad = svc
            .execute_graphql(
                r#"mutation { insertEdge(edgeType: "knows", fromType: "person", fromId: "p1", toType: "person", toId: "nope") { edgeType } }"#,
                None,
                None,
            )
            .await;
        assert!(!bad.is_success());

        // 出边/入边邻接都能查到
        let out = run(
            &svc,
            r#"{ neighbors(nodeType: "person", id: "p1", edgeType: "knows") { node { id } } }"#,
        )
        .await;
        assert!(out["neighbors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["node"]["id"] == "p3"));
        let inc = run(
            &svc,
            r#"{ neighbors(nodeType: "person", id: "p3", edgeType: "knows", direction: IN) { node { id } } }"#,
        )
        .await;
        assert!(inc["neighbors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["node"]["id"] == "p1"));

        // 更新边
        let data = run(
            &svc,
            r#"mutation { updateEdge(edgeType: "knows", fromType: "person", fromId: "p1", toType: "person", toId: "p3", properties: {weight: 0.5}) { properties } }"#,
        )
        .await;
        assert_eq!(data["updateEdge"]["properties"]["since"], 2024);
        assert_eq!(data["updateEdge"]["properties"]["weight"], 0.5);

        // 删除边
        let data = run(
            &svc,
            r#"mutation { deleteEdge(edgeType: "knows", fromType: "person", fromId: "p1", toType: "person", toId: "p3") }"#,
        )
        .await;
        assert_eq!(data["deleteEdge"], true);

        // 删除节点级联删除其关联边（p2 只有一条入边和一条出边）
        let data = run(
            &svc,
            r#"mutation { deleteNode(nodeType: "person", id: "p2") }"#,
        )
        .await;
        assert_eq!(data["deleteNode"], true);

        let stats = run(&svc, r#"{ stats { nodeCount edgeCount } }"#).await;
        assert_eq!(stats["stats"]["nodeCount"], 3);
        assert_eq!(stats["stats"]["edgeCount"], 1);
    }
}
