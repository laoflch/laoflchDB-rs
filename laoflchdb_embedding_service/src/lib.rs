pub mod proto {
    tonic::include_proto!("laoflchdb.embedding");
}

use proto::embedding_index_service_server::EmbeddingIndexService;
use proto::{
    DeleteEmbeddingRequest, DeleteEmbeddingResponse, GetIndexInfoRequest, GetIndexInfoResponse,
    IndexStats, InsertEmbeddingRequest, InsertEmbeddingResponse, SaveSnapshotRequest,
    SaveSnapshotResponse, LoadSnapshotRequest, LoadSnapshotResponse,
    SearchResult, SearchEmbeddingRequest, SearchEmbeddingResponse,
};
use anda_db_hnsw::{BoxError, DistanceMetric, HnswConfig, HnswIndex, SelectNeighborsStrategy};
use laoflchdb_engines::{EngineOptions, StorageEngine};
use laoflchdb_kv_rocksdb_engine::KVRocksDBEngine;
use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, RwLock};
use tonic::{Request, Response, Status};

/// 单个索引配置
#[derive(Debug, Clone)]
pub struct IndexConfig {
    /// 索引名称
    pub name: String,
    /// 向量维度
    pub dim: usize,
    /// HNSW max connections (M)
    pub m: u8,
    /// HNSW ef construction
    pub ef_construction: usize,
    /// HNSW ef search
    pub ef_search: usize,
    /// 最大元素数
    pub max_elements: u64,
    /// 距离度量
    pub distance_metric: DistanceMetric,
}

impl IndexConfig {
    /// 从字符串创建距离度量（支持 "cosine", "euclidean", "dot", "dotproduct"）
    pub fn distance_metric_from_str(s: &str) -> DistanceMetric {
        match s.to_lowercase().as_str() {
            "euclidean" => DistanceMetric::Euclidean,
            "dot" | "dotproduct" => DistanceMetric::InnerProduct,
            _ => DistanceMetric::Cosine,
        }
    }
}

/// 嵌入向量索引服务配置
#[derive(Debug, Clone)]
pub struct EmbeddingServiceConfig {
    /// 索引定义列表
    pub indices: Vec<IndexConfig>,
    /// KV RocksDB 数据存储路径（向量数据持久化）
    pub kv_db_path: String,
    /// HNSW 图拓扑快照保存路径
    pub snapshot_path: String,
}

impl Default for EmbeddingServiceConfig {
    fn default() -> Self {
        Self {
            indices: vec![
                IndexConfig {
                    name: "default".to_string(),
                    dim: 512,
                    m: 32,
                    ef_construction: 200,
                    ef_search: 50,
                    max_elements: 1_000_000,
                    distance_metric: DistanceMetric::Cosine,
                },
                IndexConfig {
                    name: "image".to_string(),
                    dim: 512,
                    m: 32,
                    ef_construction: 200,
                    ef_search: 50,
                    max_elements: 1_000_000,
                    distance_metric: DistanceMetric::Cosine,
                },
                IndexConfig {
                    name: "face".to_string(),
                    dim: 512,
                    m: 32,
                    ef_construction: 200,
                    ef_search: 50,
                    max_elements: 1_000_000,
                    distance_metric: DistanceMetric::Cosine,
                },
                IndexConfig {
                    name: "memory".to_string(),
                    dim: 512,
                    m: 32,
                    ef_construction: 200,
                    ef_search: 50,
                    max_elements: 1_000_000,
                    distance_metric: DistanceMetric::Cosine,
                },
            ],
            kv_db_path: "./laoflch_hnsw_data".to_string(),
            snapshot_path: "./laoflch_hnsw_snapshots".to_string(),
        }
    }
}

/// 单个索引的状态
struct IndexState {
    /// HNSW 内存索引
    index: RwLock<HnswIndex>,
    /// 该索引的配置
    config: IndexConfig,
}

/// HNSW 索引服务实现
///
/// 职责分工：
/// - `anda_db_hnsw` (HnswIndex): 管理内存中的 HNSW 图拓扑（分层可导航小世界图）
/// - `KVRocksDBEngine`: 持久化存储向量数据（RocksDB）
/// - `snapshot_path`: 图拓扑快照文件保存路径（用于启动恢复）
///
/// 支持多索引：每个索引名对应一个独立的 HNSW 图拓扑和 RocksDB 表。
pub struct EmbeddingIndexServiceImpl {
    /// 按名称索引的 HNSW 内存图集合
    indices: HashMap<String, IndexState>,
    /// 向量数据持久化存储（RocksDB）
    storage: Mutex<KVRocksDBEngine>,
    /// 服务配置
    config: Arc<EmbeddingServiceConfig>,
}

impl EmbeddingIndexServiceImpl {
    /// 创建 HNSW 索引服务实例
    ///
    /// 根据配置中的 `indices` 列表，为每个索引名创建一个独立的 HNSW 图。
    pub async fn new(config: &EmbeddingServiceConfig) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        // 确保快照目录存在
        tokio::fs::create_dir_all(&config.snapshot_path).await?;

        // 确保 KV 数据目录存在
        tokio::fs::create_dir_all(&config.kv_db_path).await?;

        // 初始化 RocksDB KV 存储引擎
        let db_path = format!("{}/hnsw_vectors", config.kv_db_path);
        let opts = EngineOptions {
            db_path: db_path.clone(),
            schema_name: "hnsw".to_string(),
        };
        let storage = KVRocksDBEngine::new(&opts)?;

        // 为每个索引定义创建 HNSW 图
        let mut indices = HashMap::new();
        for ic in &config.indices {
            let hnsw_config = HnswConfig {
                dimension: ic.dim,
                max_layers: 16,
                max_connections: ic.m,
                ef_construction: ic.ef_construction,
                ef_search: ic.ef_search,
                distance_metric: ic.distance_metric,
                scale_factor: None,
                select_neighbors_strategy: SelectNeighborsStrategy::Heuristic,
            };
            let index = HnswIndex::new(ic.name.clone(), Some(hnsw_config));
            indices.insert(ic.name.clone(), IndexState {
                index: RwLock::new(index),
                config: ic.clone(),
            });
            log::info!("HNSW 索引创建: name={}, dim={}, metric={:?}", ic.name, ic.dim, ic.distance_metric);
        }

        Ok(Self {
            indices,
            storage: Mutex::new(storage),
            config: Arc::new(config.clone()),
        })
    }

    /// 获取指定索引的配置
    fn get_index_config(&self, index_name: &str) -> Option<&IndexConfig> {
        self.indices.get(index_name).map(|s| &s.config)
    }

    /// 从快照文件恢复所有 HNSW 图拓扑（如果快照文件存在）
    ///
    /// 每个索引的独立快照文件命名: {snapshot_path}/{name}.{meta|ids|nodes}.cbor
    pub async fn try_load_snapshot(&self) -> Result<Option<u64>, Box<dyn std::error::Error + Send + Sync>> {
        let mut total = 0u64;
        for (name, state) in &self.indices {
            let meta_path = format!("{}/{}.meta.cbor", self.config.snapshot_path, name);
            let ids_path = format!("{}/{}.ids.cbor", self.config.snapshot_path, name);
            let nodes_path = format!("{}/{}.nodes.cbor", self.config.snapshot_path, name);

            if !std::path::Path::new(&meta_path).exists() {
                log::info!("索引 [{}] 未找到快照 ({}), 使用空索引启动", name, meta_path);
                continue;
            }

            let nodes_data = if std::path::Path::new(&nodes_path).exists() {
                let bytes = tokio::fs::read(&nodes_path).await?;
                let mut map = HashMap::new();
                let mut cursor = std::io::Cursor::new(&bytes);
                let mut buf_id = [0u8; 8];
                let mut buf_len = [0u8; 4];

                loop {
                    match cursor.read_exact(&mut buf_id) {
                        Ok(()) => {}
                        Err(ref e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                        Err(e) => return Err(e.into()),
                    }
                    cursor.read_exact(&mut buf_len)?;
                    let data_len = u32::from_le_bytes(buf_len) as usize;
                    let mut data = vec![0u8; data_len];
                    cursor.read_exact(&mut data)?;
                    map.insert(u64::from_le_bytes(buf_id), data);
                }
                Arc::new(map)
            } else {
                Arc::new(HashMap::new())
            };

            let meta_file = std::fs::File::open(&meta_path)?;
            let ids_file = std::fs::File::open(&ids_path)?;

            let loaded = HnswIndex::load_all(
                std::io::BufReader::new(meta_file),
                std::io::BufReader::new(ids_file),
                move |id| {
                    let map = nodes_data.clone();
                    async move { Ok(map.get(&id).cloned()) }
                },
            )
            .await?;

            let stats = loaded.stats();
            let mut index = state.index.write().await;
            *index = loaded;
            total += stats.num_elements;

            log::info!(
                "索引 [{}] 快照加载成功: {} 条向量",
                name, stats.num_elements
            );
        }

        if total > 0 {
            Ok(Some(total))
        } else {
            Ok(None)
        }
    }

    /// 确保索引的 KV 表存在
    async fn ensure_table(&self, index_name: &str) -> Result<(), Status> {
        let table_name = format!("hnsw_{}", index_name);
        let storage = self.storage.lock().await;
        if !storage.table_exists(&table_name) {
            drop(storage);
            let mut storage = self.storage.lock().await;
            storage.create_table(&table_name, None, &[]).await.map_err(|e| {
                Status::internal(format!("创建表失败: {}", e))
            })?;
        }
        Ok(())
    }

    fn unix_ms() -> u64 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    /// 保存所有索引的快照：metadata + ids + dirty nodes（每个索引独立文件）
    async fn save_snapshot_internal(&self) -> Result<String, Status> {
        tokio::fs::create_dir_all(&self.config.snapshot_path)
            .await
            .map_err(|e| Status::internal(format!("创建快照目录失败: {}", e)))?;

        let ts = Self::unix_ms();

        for (name, state) in &self.indices {
            let stem = format!("{}/{}", self.config.snapshot_path, name);
            let meta_path = format!("{}.meta.cbor", stem);
            let ids_path = format!("{}.ids.cbor", stem);
            let nodes_path = format!("{}.nodes.cbor", stem);

            let guard = state.index.read().await;

            // 1. 保存 metadata
            {
                let file = std::fs::File::create(&meta_path).map_err(|e| {
                    Status::internal(format!("创建 metadata 文件失败: {}", e))
                })?;
                guard
                    .store_metadata(std::io::BufWriter::new(file), ts)
                    .map_err(|e| Status::internal(format!("保存 metadata 失败: {}", e)))?;
            }

            // 2. 保存 IDs
            {
                let file = std::fs::File::create(&ids_path).map_err(|e| {
                    Status::internal(format!("创建 ids 文件失败: {}", e))
                })?;
                guard
                    .store_ids(std::io::BufWriter::new(file))
                    .map_err(|e| Status::internal(format!("保存 ids 失败: {}", e)))?;
            }

            // 3. 保存 dirty nodes
            {
                let file = tokio::fs::File::create(&nodes_path)
                    .await
                    .map_err(|e| Status::internal(format!("创建 nodes 文件失败: {}", e)))?;
                let writer = Arc::new(tokio::sync::Mutex::new(file));

                guard
                    .store_dirty_nodes({
                        let w = writer.clone();
                        move |id: u64, data: &[u8]| {
                            let w = w.clone();
                            let owned_data = data.to_vec();
                            async move {
                                let mut f = w.lock().await;
                                f.write_all(&id.to_le_bytes())
                                    .await
                                    .map_err(|e| Box::new(e) as BoxError)?;
                                f.write_all(&(owned_data.len() as u32).to_le_bytes())
                                    .await
                                    .map_err(|e| Box::new(e) as BoxError)?;
                                f.write_all(&owned_data)
                                    .await
                                    .map_err(|e| Box::new(e) as BoxError)?;
                                Ok(true)
                            }
                        }
                    })
                    .await
                    .map_err(|e| Status::internal(format!("保存 nodes 失败: {}", e)))?;
            }
        }

        // 保存 JSON 元数据
        let meta_json = serde_json::json!({
            "indices": self.config.indices.iter().map(|ic| {
                serde_json::json!({
                    "name": ic.name,
                    "dim": ic.dim,
                    "m": ic.m,
                    "ef_construction": ic.ef_construction,
                    "ef_search": ic.ef_search,
                    "max_elements": ic.max_elements,
                    "distance_metric": format!("{:?}", ic.distance_metric),
                })
            }).collect::<Vec<_>>(),
            "saved_at": ts,
        });
        let meta_json_path = format!("{}/hnsw_meta.json", self.config.snapshot_path);
        tokio::fs::write(&meta_json_path, serde_json::to_string_pretty(&meta_json).unwrap())
            .await
            .map_err(|e| Status::internal(format!("保存 JSON 元数据失败: {}", e)))?;

        log::info!("所有 HNSW 索引快照已保存: {}", self.config.snapshot_path);
        Ok(self.config.snapshot_path.clone())
    }

    /// 加载所有索引的快照
    async fn load_snapshot_internal(&self) -> Result<u64, Status> {
        let mut total = 0u64;
        for (name, state) in &self.indices {
            let meta_path = format!("{}/{}.meta.cbor", self.config.snapshot_path, name);
            let ids_path = format!("{}/{}.ids.cbor", self.config.snapshot_path, name);
            let nodes_path = format!("{}/{}.nodes.cbor", self.config.snapshot_path, name);

            if !std::path::Path::new(&meta_path).exists() {
                log::warn!("索引 [{}] 快照文件不存在: {}", name, meta_path);
                continue;
            }

            let nodes_data = {
                let bytes = tokio::fs::read(&nodes_path)
                    .await
                    .map_err(|e| Status::not_found(format!("nodes 文件不存在: {} ({})", nodes_path, e)))?;
                let mut map = HashMap::new();
                let mut cursor = std::io::Cursor::new(&bytes);
                let mut buf_id = [0u8; 8];
                let mut buf_len = [0u8; 4];

                loop {
                    match cursor.read_exact(&mut buf_id) {
                        Ok(()) => {}
                        Err(ref e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                        Err(e) => return Err(Status::internal(format!("解析 nodes 数据失败: {}", e))),
                    }
                    cursor.read_exact(&mut buf_len)
                        .map_err(|e| Status::internal(format!("解析 nodes 长度失败: {}", e)))?;
                    let data_len = u32::from_le_bytes(buf_len) as usize;
                    let mut data = vec![0u8; data_len];
                    cursor.read_exact(&mut data)
                        .map_err(|e| Status::internal(format!("解析 nodes 内容失败: {}", e)))?;
                    map.insert(u64::from_le_bytes(buf_id), data);
                }
                Arc::new(map)
            };

            let meta_file = std::fs::File::open(&meta_path)
                .map_err(|e| Status::not_found(format!("meta 文件不存在: {} ({})", meta_path, e)))?;
            let ids_file = std::fs::File::open(&ids_path)
                .map_err(|e| Status::not_found(format!("ids 文件不存在: {} ({})", ids_path, e)))?;

            let loaded = HnswIndex::load_all(
                std::io::BufReader::new(meta_file),
                std::io::BufReader::new(ids_file),
                move |id| {
                    let map = nodes_data.clone();
                    async move { Ok(map.get(&id).cloned()) }
                },
            )
            .await
            .map_err(|e| Status::internal(format!("加载快照失败: {}", e)))?;

            let stats = loaded.stats();
            let mut index = state.index.write().await;
            *index = loaded;
            total += stats.num_elements;

            log::info!("索引 [{}] 快照加载成功: {} 条向量", name, stats.num_elements);
        }

        if total == 0 {
            return Err(Status::not_found(format!("未找到任何快照: {}", self.config.snapshot_path)));
        }
        Ok(total)
    }
}

/// 允许通过 `Arc<EmbeddingIndexServiceImpl>` 直接作为 gRPC 服务注册
#[tonic::async_trait]
impl proto::embedding_index_service_server::EmbeddingIndexService
    for std::sync::Arc<EmbeddingIndexServiceImpl>
{
    async fn insert_embedding(
        &self,
        request: tonic::Request<InsertEmbeddingRequest>,
    ) -> std::result::Result<tonic::Response<InsertEmbeddingResponse>, tonic::Status> {
        self.as_ref().insert_embedding(request).await
    }

    async fn search_embedding(
        &self,
        request: tonic::Request<SearchEmbeddingRequest>,
    ) -> std::result::Result<tonic::Response<SearchEmbeddingResponse>, tonic::Status> {
        self.as_ref().search_embedding(request).await
    }

    async fn delete_embedding(
        &self,
        request: tonic::Request<DeleteEmbeddingRequest>,
    ) -> std::result::Result<tonic::Response<DeleteEmbeddingResponse>, tonic::Status> {
        self.as_ref().delete_embedding(request).await
    }

    async fn get_index_info(
        &self,
        request: tonic::Request<GetIndexInfoRequest>,
    ) -> std::result::Result<tonic::Response<GetIndexInfoResponse>, tonic::Status> {
        self.as_ref().get_index_info(request).await
    }

    async fn save_snapshot(
        &self,
        request: tonic::Request<SaveSnapshotRequest>,
    ) -> std::result::Result<tonic::Response<SaveSnapshotResponse>, tonic::Status> {
        self.as_ref().save_snapshot(request).await
    }

    async fn load_snapshot(
        &self,
        request: tonic::Request<LoadSnapshotRequest>,
    ) -> std::result::Result<tonic::Response<LoadSnapshotResponse>, tonic::Status> {
        self.as_ref().load_snapshot(request).await
    }
}

#[tonic::async_trait]
impl EmbeddingIndexService for EmbeddingIndexServiceImpl {
    /// 插入向量到指定名称的 HNSW 索引
    async fn insert_embedding(
        &self,
        request: Request<InsertEmbeddingRequest>,
    ) -> Result<Response<InsertEmbeddingResponse>, Status> {
        let req = request.into_inner();
        let index_name = if req.index_name.is_empty() {
            "default"
        } else {
            &req.index_name
        };

        // 查找索引
        let state = self.indices.get(index_name).ok_or_else(|| {
            Status::not_found(format!("索引不存在: {}", index_name))
        })?;
        let dim = state.config.dim;

        // 维度校验
        if req.embedding.len() != dim {
            return Ok(Response::new(InsertEmbeddingResponse {
                success: false,
                message: format!("向量维度不匹配: 索引名={}, 需要 {}, 实际 {}", index_name, dim, req.embedding.len()),
            }));
        }

        // 1. 写入 KV RocksDB（持久化向量数据）
        self.ensure_table(index_name).await?;
        let table_name = format!("hnsw_{}", index_name);
        let key = format!("v:{}", req.id).into_bytes();
        let value: Vec<u8> = req.embedding.iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();

        {
            let mut storage = self.storage.lock().await;
            storage.put(&table_name, &key, &value).await.map_err(|e| {
                Status::internal(format!("写入存储失败: {}", e))
            })?;
        }

        // 2. 插入 HNSW 内存索引（构建图拓扑）
        let ts = Self::unix_ms();
        let index = state.index.read().await;
        index.insert_f32(req.id, req.embedding, ts).map_err(|e| {
            Status::internal(format!("HNSW 插入失败: {}", e))
        })?;

        log::info!("向量插入成功: id={}, index={}", req.id, index_name);
        Ok(Response::new(InsertEmbeddingResponse {
            success: true,
            message: format!("向量插入成功, id={}, index={}", req.id, index_name),
        }))
    }

    /// 搜索最近邻向量
    async fn search_embedding(
        &self,
        request: Request<SearchEmbeddingRequest>,
    ) -> Result<Response<SearchEmbeddingResponse>, Status> {
        let req = request.into_inner();
        let index_name = if req.index_name.is_empty() {
            "default"
        } else {
            &req.index_name
        };
        let top_k = if req.top_k <= 0 { 10 } else { req.top_k as usize };

        if req.query_embedding.is_empty() {
            return Ok(Response::new(SearchEmbeddingResponse {
                success: false,
                message: "查询向量为空".to_string(),
                results: vec![],
            }));
        }

        // 查找索引
        let state = self.indices.get(index_name).ok_or_else(|| {
            Status::not_found(format!("索引不存在: {}", index_name))
        })?;

        // 执行 HNSW ANN 搜索
        let results = {
            let index = state.index.read().await;
            index.search_f32(&req.query_embedding, top_k).map_err(|e| {
                Status::internal(format!("HNSW 搜索失败: {}", e))
            })?
        };

        if results.is_empty() {
            return Ok(Response::new(SearchEmbeddingResponse {
                success: true,
                message: "未找到匹配结果".to_string(),
                results: vec![],
            }));
        }

        // 从 KV RocksDB 加载向量数据
        let table_name = format!("hnsw_{}", index_name);

        let mut search_results = Vec::with_capacity(results.len());
        for (id, distance) in &results {
            let vector = {
                let storage = self.storage.lock().await;
                let key = format!("v:{}", id).into_bytes();
                storage.get(&table_name, &key).await.ok().flatten().map(|bytes| {
                    bytes
                        .chunks_exact(4)
                        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                        .collect()
                })
            };

            search_results.push(SearchResult {
                id: *id,
                distance: *distance,
                embedding: vector.unwrap_or_default(),
            });
        }

        Ok(Response::new(SearchEmbeddingResponse {
            success: true,
            message: format!("搜索完成, 返回 {} 条结果", search_results.len()),
            results: search_results,
        }))
    }

    /// 删除向量
    async fn delete_embedding(
        &self,
        request: Request<DeleteEmbeddingRequest>,
    ) -> Result<Response<DeleteEmbeddingResponse>, Status> {
        let req = request.into_inner();
        let index_name = if req.index_name.is_empty() {
            "default"
        } else {
            &req.index_name
        };

        // 查找索引
        let state = self.indices.get(index_name).ok_or_else(|| {
            Status::not_found(format!("索引不存在: {}", index_name))
        })?;

        // 1. 从 HNSW 索引删除
        let ts = Self::unix_ms();
        let removed = {
            let index = state.index.read().await;
            index.remove(req.id, ts)
        };

        // 2. 从 KV RocksDB 删除
        let table_name = format!("hnsw_{}", index_name);
        let key = format!("v:{}", req.id).into_bytes();
        {
            let mut storage = self.storage.lock().await;
            let _ = storage.delete(&table_name, &key).await;
        }

        if removed {
            log::info!("向量删除成功: id={}, index={}", req.id, index_name);
            Ok(Response::new(DeleteEmbeddingResponse {
                success: true,
                message: format!("向量删除成功, id={}, index={}", req.id, index_name),
            }))
        } else {
            Ok(Response::new(DeleteEmbeddingResponse {
                success: false,
                message: format!("未找到向量 id={}, index={}", req.id, index_name),
            }))
        }
    }

    /// 获取指定索引的统计信息
    async fn get_index_info(
        &self,
        request: Request<GetIndexInfoRequest>,
    ) -> Result<Response<GetIndexInfoResponse>, Status> {
        let req = request.into_inner();

        // index_name 为空时返回所有索引
        if req.index_name.is_empty() {
            let mut all_stats = Vec::new();
            let mut total_elements = 0u64;
            for (name, state) in &self.indices {
                let stats = {
                    let index = state.index.read().await;
                    index.stats()
                };
                total_elements += stats.num_elements;
                all_stats.push(IndexStats {
                    num_elements: stats.num_elements,
                    max_layers: stats.max_layer as u32,
                    avg_connections: 0.0,
                    search_count: stats.search_count,
                    insert_count: stats.insert_count,
                    delete_count: stats.delete_count,
                    dim: state.config.dim as u32,
                    distance_metric: format!("{:?}", state.config.distance_metric),
                    snapshot_path: self.config.snapshot_path.clone(),
                    name: name.clone(),
                });
            }
            let main_stats = all_stats.first().cloned().unwrap_or_default();
            return Ok(Response::new(GetIndexInfoResponse {
                success: true,
                message: format!("索引数: {}, 总向量数: {}", self.indices.len(), total_elements),
                stats: Some(main_stats),
                all_stats,
            }));
        }

        let state = self.indices.get(&req.index_name).ok_or_else(|| {
            Status::not_found(format!("索引不存在: {}", req.index_name))
        })?;

        let stats = {
            let index = state.index.read().await;
            index.stats()
        };

        Ok(Response::new(GetIndexInfoResponse {
            success: true,
            message: format!("索引: {}, 向量数: {}", req.index_name, stats.num_elements),
            stats: Some(IndexStats {
                num_elements: stats.num_elements,
                max_layers: stats.max_layer as u32,
                avg_connections: 0.0,
                search_count: stats.search_count,
                insert_count: stats.insert_count,
                delete_count: stats.delete_count,
                dim: state.config.dim as u32,
                distance_metric: format!("{:?}", state.config.distance_metric),
                snapshot_path: self.config.snapshot_path.clone(),
                name: req.index_name.clone(),
            }),
            all_stats: vec![],
        }))
    }

    /// 保存所有 HNSW 图拓扑快照
    async fn save_snapshot(
        &self,
        _request: Request<SaveSnapshotRequest>,
    ) -> Result<Response<SaveSnapshotResponse>, Status> {
        let path = self.save_snapshot_internal().await?;
        Ok(Response::new(SaveSnapshotResponse {
            success: true,
            message: format!("所有索引快照已保存: {}", path),
            path,
        }))
    }

    /// 加载所有 HNSW 图拓扑快照
    async fn load_snapshot(
        &self,
        _request: Request<LoadSnapshotRequest>,
    ) -> Result<Response<LoadSnapshotResponse>, Status> {
        let num_elements = self.load_snapshot_internal().await?;
        Ok(Response::new(LoadSnapshotResponse {
            success: true,
            message: format!("所有索引快照加载成功: {} 条向量", num_elements),
            num_elements,
        }))
    }
}