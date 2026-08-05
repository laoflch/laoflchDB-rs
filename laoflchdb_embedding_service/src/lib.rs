pub mod proto {
    tonic::include_proto!("laoflchdb.embedding");
}
pub mod storage_proto {
    tonic::include_proto!("laoflchdb.embedding_storage");
}

use proto::embedding_index_service_server::EmbeddingIndexService;
use proto::{
    DeleteEmbeddingRequest, DeleteEmbeddingResponse, EmbeddingEntry, GetIndexInfoRequest,
    GetIndexInfoResponse, IndexStats, InsertEmbeddingRequest, InsertEmbeddingResponse,
    ListEmbeddingsRequest, ListEmbeddingsResponse, SaveSnapshotRequest, SaveSnapshotResponse,
    LoadSnapshotRequest, LoadSnapshotResponse,
    AnalyzeConsistencyRequest, AnalyzeConsistencyResponse, RebuildIndexFromRocksDbRequest,
    RebuildIndexFromRocksDbResponse,
    SearchResult, SearchEmbeddingRequest, SearchEmbeddingResponse,
};
use anda_db_hnsw::{BoxError, DistanceMetric, HnswConfig, HnswIndex, SelectNeighborsStrategy};
use laoflchdb_engines::{EngineOptions, StorageEngine};
use laoflchdb_kv_rocksdb_engine::KVRocksDBEngine;
use std::collections::HashMap;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::SystemTime;
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, RwLock};
use tonic::{Request, Response, Status};
use prost::Message;

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
    /// 存储格式: "raw"（旧格式）或 "protobuf"（新格式）
    pub storage_format: String,
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

    /// 存储格式是否为 protobuf
    pub fn is_protobuf_format(&self) -> bool {
        self.storage_format == "protobuf"
    }

    /// 转换为 HnswConfig
    pub fn hnsw_config(&self) -> Result<HnswConfig, Status> {
        Ok(HnswConfig {
            dimension: self.dim,
            max_layers: 16u8,
            max_connections: self.m as u8,
            ef_construction: self.ef_construction,
            ef_search: self.ef_search,
            distance_metric: self.distance_metric,
            scale_factor: None,
            select_neighbors_strategy: SelectNeighborsStrategy::Heuristic,
        })
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
    /// 启动模式: "snapshot"（快照恢复）或 "rebuild"（从 RocksDB 重建）
    pub startup_mode: String,
    /// 搜索过滤的最大候选倍数上限
    pub max_filter_multiplier: f32,
    /// 搜索过滤的最大迭代次数上限
    pub max_filter_iterations: usize,
    /// 搜索结果的最大距离上限
    pub max_search_distance: f32,
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
                    storage_format: "raw".to_string(),
                },
                IndexConfig {
                    name: "image".to_string(),
                    dim: 512,
                    m: 32,
                    ef_construction: 200,
                    ef_search: 50,
                    max_elements: 1_000_000,
                    distance_metric: DistanceMetric::Cosine,
                    storage_format: "raw".to_string(),
                },
                IndexConfig {
                    name: "face".to_string(),
                    dim: 512,
                    m: 32,
                    ef_construction: 200,
                    ef_search: 50,
                    max_elements: 1_000_000,
                    distance_metric: DistanceMetric::Cosine,
                    storage_format: "raw".to_string(),
                },
                IndexConfig {
                    name: "memory".to_string(),
                    dim: 512,
                    m: 32,
                    ef_construction: 200,
                    ef_search: 50,
                    max_elements: 1_000_000,
                    distance_metric: DistanceMetric::Cosine,
                    storage_format: "raw".to_string(),
                },
            ],
            kv_db_path: "./laoflch_hnsw_data".to_string(),
            snapshot_path: "./laoflch_hnsw_snapshots".to_string(),
            startup_mode: "snapshot".to_string(),
            max_filter_multiplier: 20.0,
            max_filter_iterations: 10,
            max_search_distance: f32::INFINITY,
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
    /// 是否正在构建/重建索引（启动时重建或手动重建期间为 true）
    building: Arc<AtomicBool>,
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
            building: Arc::new(AtomicBool::new(false)),
        })
    }

    /// 获取指定索引的配置
    fn get_index_config(&self, index_name: &str) -> Option<&IndexConfig> {
        self.indices.get(index_name).map(|s| &s.config)
    }

    /// 检查索引是否正在构建中
    pub fn is_building(&self) -> bool {
        self.building.load(Ordering::Relaxed)
    }

    /// 设置构建状态
    fn set_building(&self, val: bool) {
        self.building.store(val, Ordering::Relaxed);
    }

    /// 检查构建状态，如果正在构建中则返回错误
    fn check_building(&self) -> Result<(), Status> {
        if self.building.load(Ordering::Relaxed) {
            Err(Status::unavailable("HNSW 服务正在构建中，请稍后再试"))
        } else {
            Ok(())
        }
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

    /// 保存所有索引的快照（公开方法，供服务退出时调用）
    pub async fn save_snapshot_on_shutdown(&self) -> Result<String, Status> {
        self.save_snapshot_internal().await
    }

    /// 从 RocksDB 重建所有索引（公开方法，供外部调用）
    /// 设置 building 标志，重建完成后自动保存快照
    pub async fn rebuild_all_from_rocks_db_public(&self) -> Result<u64, Status> {
        self.set_building(true);
        let result = self.rebuild_all_from_rocks_db_internal().await;
        self.set_building(false);
        let _ = self.save_snapshot_internal().await;
        result
    }

    /// 内部重建方法（不设置 building 标志）
    /// 遍历所有索引，从 RocksDB 逐条重建 HNSW 图拓扑
    async fn rebuild_all_from_rocks_db_internal(&self) -> Result<u64, Status> {
        let mut total = 0u64;
        let index_names: Vec<String> = self.indices.keys().cloned().collect();
        for name in index_names {
            let req = Request::new(RebuildIndexFromRocksDbRequest {
                index_name: name.clone(),
            });
            match self.rebuild_index_from_rocks_db(req).await {
                Ok(resp) => {
                    let inner = resp.into_inner();
                    total += inner.rebuilt_count;
                    log::info!("重建索引 [{}]: {} 条向量", name, inner.rebuilt_count);
                }
                Err(e) => {
                    log::warn!("重建索引 [{}] 失败: {}", name, e);
                }
            }
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

    async fn list_embeddings(
        &self,
        request: tonic::Request<ListEmbeddingsRequest>,
    ) -> std::result::Result<tonic::Response<ListEmbeddingsResponse>, tonic::Status> {
        self.as_ref().list_embeddings(request).await
    }

    async fn analyze_consistency(
        &self,
        request: tonic::Request<AnalyzeConsistencyRequest>,
    ) -> std::result::Result<tonic::Response<AnalyzeConsistencyResponse>, tonic::Status> {
        self.as_ref().analyze_consistency(request).await
    }

    async fn rebuild_index_from_rocks_db(
        &self,
        request: tonic::Request<RebuildIndexFromRocksDbRequest>,
    ) -> std::result::Result<tonic::Response<RebuildIndexFromRocksDbResponse>, tonic::Status> {
        self.as_ref().rebuild_index_from_rocks_db(request).await
    }
}

#[tonic::async_trait]
impl EmbeddingIndexService for EmbeddingIndexServiceImpl {
    /// 插入向量到指定名称的 HNSW 索引
    async fn insert_embedding(
        &self,
        request: Request<InsertEmbeddingRequest>,
    ) -> Result<Response<InsertEmbeddingResponse>, Status> {
        self.check_building()?;
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

        // 1. 写入 KV RocksDB（持久化向量数据+元数据）
        self.ensure_table(index_name).await?;
        let table_name = format!("hnsw_{}", index_name);
        let key = format!("v:{}", req.id).into_bytes();
        
        let is_protobuf = state.config.is_protobuf_format();
        let value: Vec<u8> = if is_protobuf {
            // protobuf 格式：存储向量 + 过滤字段
            let ts = Self::unix_ms() as i64;
            let stored_embedding = storage_proto::StoredEmbedding {
                embedding: req.embedding.clone(),
                fields: req.fields,
                created_at: ts,
                updated_at: ts,
            };
            let mut buf = Vec::new();
            prost::Message::encode(&stored_embedding, &mut buf).map_err(|e| {
                Status::internal(format!("序列化存储数据失败: {}", e))
            })?;
            buf
        } else {
            // 旧格式：直接写入 f32 字节
            let bytes: Vec<u8> = req.embedding.iter()
                .flat_map(|f| f.to_le_bytes())
                .collect();
            bytes
        };

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
        self.check_building()?;
        let req = request.into_inner();
        let index_name = if req.index_name.is_empty() {
            "default"
        } else {
            &req.index_name
        };
        let top_k = if req.top_k <= 0 { 10 } else { req.top_k as usize };
        let has_filters = !req.field_filters.is_empty();
        let config = &*self.config;
        
        // 使用请求参数，但受配置上限限制
        let filter_multiplier = {
            let req_val = if req.filter_multiplier > 0.0 { req.filter_multiplier } else { 5.0 };
            let max_val = config.max_filter_multiplier;
            if max_val > 0.0 { req_val.min(max_val) } else { req_val }
        };
        
        let max_iterations = {
            let req_val = if req.max_filter_iterations > 0 { req.max_filter_iterations as usize } else { 3 };
            let max_val = config.max_filter_iterations;
            if max_val > 0 { req_val.min(max_val) } else { req_val }
        };
        
        let max_distance = {
            let req_val = if req.max_distance > 0.0 { req.max_distance } else { f32::INFINITY };
            let max_val = config.max_search_distance;
            if max_val > 0.0 { req_val.min(max_val) } else { req_val }
        };

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

        // 无过滤条件时，直接搜索 top_k
        if !has_filters {
            let results = {
                let index = state.index.read().await;
                index.search_f32(&req.query_embedding, top_k).map_err(|e| {
                    Status::internal(format!("HNSW 搜索失败: {}", e))
                })?
            };

            let table_name = format!("hnsw_{}", index_name);
            let storage = self.storage.lock().await;
            
            let mut search_results = Vec::with_capacity(results.len());
            for (id, distance) in &results {
                // 检查距离限制
                if *distance > max_distance {
                    log::info!("距离超过限制，停止搜索: {} > {}", distance, max_distance);
                    break;
                }

                let key = format!("v:{}", id).into_bytes();
                let embedding = storage.get(&table_name, &key).await.ok().flatten().map(|bytes| {
                    if let Ok(stored_embedding) = storage_proto::StoredEmbedding::decode(bytes.as_slice()) {
                        stored_embedding.embedding
                    } else {
                        // 兼容旧格式
                        bytes
                            .chunks_exact(4)
                            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                            .collect()
                    }
                });

                search_results.push(SearchResult {
                    id: *id,
                    distance: *distance,
                    embedding: embedding.unwrap_or_default(),
                });
            }
            drop(storage);

            return Ok(Response::new(SearchEmbeddingResponse {
                success: true,
                message: format!("搜索完成, 返回 {} 条结果", search_results.len()),
                results: search_results,
            }));
        }

        // 有过滤条件时，循环搜索直到找到足够的结果或达到最大迭代次数
        let table_name = format!("hnsw_{}", index_name);
        let storage = self.storage.lock().await;
        
        let mut search_results = Vec::with_capacity(top_k);
        let mut current_multiplier = 1.0;
        let mut seen_ids = std::collections::HashSet::new();
        let mut should_stop = false;

        for iter in 0..max_iterations {
            if should_stop {
                log::info!("已达到距离限制，停止迭代");
                break;
            }

            let search_candidate_count = (top_k as f32 * filter_multiplier * current_multiplier).ceil() as usize;
            
            log::info!("过滤迭代 {}/{}, 搜索候选数: {}", iter + 1, max_iterations, search_candidate_count);
            
            let results = {
                let index = state.index.read().await;
                index.search_f32(&req.query_embedding, search_candidate_count).map_err(|e| {
                    Status::internal(format!("HNSW 搜索失败: {}", e))
                })?
            };

            for (id, distance) in &results {
                // 检查距离限制
                if *distance > max_distance {
                    log::info!("距离超过限制，停止搜索: {} > {}", distance, max_distance);
                    should_stop = true;
                    break;
                }

                if search_results.len() >= top_k {
                    break;
                }

                if seen_ids.contains(id) {
                    continue;
                }
                seen_ids.insert(*id);

                let key = format!("v:{}", id).into_bytes();
                if let Some(bytes) = storage.get(&table_name, &key).await.ok().flatten() {
                    // 解析 StoredEmbedding
                    let stored_embedding = match storage_proto::StoredEmbedding::decode(bytes.as_slice()) {
                        Ok(se) => se,
                        Err(e) => {
                            log::warn!("解码存储数据失败 id={}: {}", id, e);
                            continue;
                        }
                    };

                    // 检查过滤条件
                    let mut matches = true;
                    for (k, v) in &req.field_filters {
                        if let Some(stored_val) = stored_embedding.fields.get(k) {
                            if stored_val != v {
                                matches = false;
                                break;
                            }
                        } else {
                            // 字段不存在，不匹配
                            matches = false;
                            break;
                        }
                    }

                    if !matches {
                        continue;
                    }

                    // 添加到结果
                    search_results.push(SearchResult {
                        id: *id,
                        distance: *distance,
                        embedding: stored_embedding.embedding,
                    });
                }
            }

            if search_results.len() >= top_k {
                log::info!("找到足够的结果，停止迭代");
                break;
            }

            current_multiplier *= 2.0;
        }
        drop(storage);

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
        self.check_building()?;
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
            let index = state.index.write().await;
            index.remove(req.id, ts)
        };

        // 2. 从 KV RocksDB 删除（必须执行，保证两者一致）
        let table_name = format!("hnsw_{}", index_name);
        let key = format!("v:{}", req.id).into_bytes();
        {
            let mut storage = self.storage.lock().await;
            if let Err(e) = storage.delete(&table_name, &key).await {
                log::error!("删除 RocksDB 向量失败: id={}, index={}, error={}", req.id, index_name, e);
            }
        }

        log::info!("向量删除处理完成: id={}, index={}, removed_from_hnsw={}", req.id, index_name, removed);

        Ok(Response::new(DeleteEmbeddingResponse {
            success: true,
            message: format!("向量删除处理完成, id={}, index={}", req.id, index_name),
        }))
    }

    /// 获取指定索引的统计信息
    async fn get_index_info(
        &self,
        request: Request<GetIndexInfoRequest>,
    ) -> Result<Response<GetIndexInfoResponse>, Status> {
        self.check_building()?;
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
        self.check_building()?;
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
        self.check_building()?;
        let num_elements = self.load_snapshot_internal().await?;
        Ok(Response::new(LoadSnapshotResponse {
            success: true,
            message: format!("所有索引快照加载成功: {} 条向量", num_elements),
            num_elements,
        }))
    }

    /// 列出索引中的所有向量
    async fn list_embeddings(
        &self,
        request: Request<ListEmbeddingsRequest>,
    ) -> Result<Response<ListEmbeddingsResponse>, Status> {
        self.check_building()?;
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
        let is_protobuf = state.config.is_protobuf_format();

        let table_name = format!("hnsw_{}", index_name);
        self.ensure_table(index_name).await?;

        let limit = if req.limit > 0 { Some(req.limit as usize) } else { None };
        let offset = req.offset.max(0) as usize;

        let start_key = b"v:".to_vec();
        let end_key = b"v:\xff".to_vec();

        let storage = self.storage.lock().await;
        let all_entries = storage.scan_range(&table_name, &start_key, &end_key, None).map_err(|e| {
            Status::internal(format!("扫描存储失败: {}", e))
        })?;
        drop(storage);

        let total = all_entries.len() as u64;

        // 应用 offset 和 limit
        let paginated: Vec<_> = all_entries.into_iter()
            .skip(offset)
            .take(limit.unwrap_or(usize::MAX))
            .collect();

        let mut entries = Vec::with_capacity(paginated.len());
        for (key, value) in paginated {
            // key format: "v:{id}" → parse id
            let id_str = String::from_utf8_lossy(&key);
            let id: u64 = id_str.strip_prefix("v:").and_then(|s| s.parse().ok()).unwrap_or(0);

            // 解析 embedding（根据存储格式）
            let embedding: Vec<f32> = if is_protobuf {
                match storage_proto::StoredEmbedding::decode(value.as_slice()) {
                    Ok(se) => se.embedding,
                    Err(e) => {
                        log::warn!("解码 protobuf 存储数据失败 id={}: {}", id, e);
                        continue;
                    }
                }
            } else {
                // 旧格式：直接解析 f32 字节
                if value.len() % 4 != 0 {
                    log::warn!("跳过无效的 value, id={}, value_len={} (不是 4 的倍数)", id, value.len());
                    continue;
                }
                value
                    .chunks_exact(4)
                    .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .collect()
            };

            entries.push(EmbeddingEntry {
                id,
                embedding,
            });
        }

        Ok(Response::new(ListEmbeddingsResponse {
            success: true,
            message: format!("获取到 {} 条向量", entries.len()),
            entries,
            total,
        }))
    }

    /// 分析 RocksDB 和 HNSW 索引的一致性
    async fn analyze_consistency(
        &self,
        request: Request<AnalyzeConsistencyRequest>,
    ) -> Result<Response<AnalyzeConsistencyResponse>, Status> {
        self.check_building()?;
        let req = request.into_inner();
        let index_name = if req.index_name.is_empty() {
            "default"
        } else {
            &req.index_name
        };

        let state = self.indices.get(index_name).ok_or_else(|| {
            Status::not_found(format!("索引不存在: {}", index_name))
        })?;

        // 1. 获取 HNSW 里的所有 ID
        let hnsw_ids: Vec<u64> = {
            let index = state.index.read().await;
            index.node_ids()
        };

        // 2. 获取 RocksDB 里的所有 ID
        let table_name = format!("hnsw_{}", index_name);
        let start_key = b"v:".to_vec();
        let end_key = b"v:\xff".to_vec();

        let storage = self.storage.lock().await;
        let all_entries = storage.scan_range(&table_name, &start_key, &end_key, None).map_err(|e| {
            Status::internal(format!("扫描 RocksDB 失败: {}", e))
        })?;
        drop(storage);

        let mut rocksdb_ids: Vec<u64> = Vec::new();
        for (key, _) in all_entries {
            let id_str = String::from_utf8_lossy(&key);
            if let Some(id) = id_str.strip_prefix("v:").and_then(|s| s.parse().ok()) {
                rocksdb_ids.push(id);
            }
        }

        // 3. 比较差异
        let hnsw_set: std::collections::HashSet<u64> = hnsw_ids.iter().cloned().collect();
        let rocksdb_set: std::collections::HashSet<u64> = rocksdb_ids.iter().cloned().collect();

        let only_in_hnsw: Vec<u64> = hnsw_set.difference(&rocksdb_set).cloned().collect();
        let only_in_rocksdb: Vec<u64> = rocksdb_set.difference(&hnsw_set).cloned().collect();

        Ok(Response::new(AnalyzeConsistencyResponse {
            success: true,
            message: format!("一致性分析完成, 不一致: HNSW({}) RocksDB({})", only_in_hnsw.len(), only_in_rocksdb.len()),
            hnsw_count: hnsw_ids.len() as u64,
            rocksdb_count: rocksdb_ids.len() as u64,
            only_in_hnsw,
            only_in_rocksdb,
        }))
    }

    /// 从 RocksDB 重建 HNSW 索引
    async fn rebuild_index_from_rocks_db(
        &self,
        request: Request<RebuildIndexFromRocksDbRequest>,
    ) -> Result<Response<RebuildIndexFromRocksDbResponse>, Status> {
        let req = request.into_inner();
        let index_name = if req.index_name.is_empty() {
            "default"
        } else {
            &req.index_name
        };

        let state = self.indices.get(index_name).ok_or_else(|| {
            Status::not_found(format!("索引不存在: {}", index_name))
        })?;

        // 1. 创建新索引（空索引）
        let new_index = Arc::new(HnswIndex::new(
            index_name.to_string(),
            Some(state.config.hnsw_config()?),
        ));

        // 2. 从 RocksDB 读取所有向量
        let table_name = format!("hnsw_{}", index_name);
        let start_key = b"v:".to_vec();
        let end_key = b"v:\xff".to_vec();

        let storage = self.storage.lock().await;
        let all_entries = storage
            .scan_range(&table_name, &start_key, &end_key, None)
            .map_err(|e| Status::internal(format!("扫描 RocksDB 失败: {}", e)))?;
        drop(storage);

        let entry_count = all_entries.len();
        log::info!("重建索引 [{}]: 从 RocksDB 读取了 {} 条向量", index_name, entry_count);

        // 3. 在 blocking 线程池中执行 CPU 密集的 vector 插入操作
        let index_dim = state.config.dim;
        let is_protobuf = state.config.is_protobuf_format();
        let index_name_clone = index_name.to_string();
        let new_index_clone = new_index.clone();
        let total_entries = all_entries.len();
        let rebuilt_count = tokio::task::spawn_blocking(move || {
            let mut count = 0u64;
            let mut last_log_ts = SystemTime::now();
            
            for (i, (key, value)) in all_entries.into_iter().enumerate() {
                // 每 1000 条或每 5 秒输出一次进度日志
                let now = SystemTime::now();
                if i % 1000 == 0 || now.duration_since(last_log_ts).map(|d| d.as_secs() > 5).unwrap_or(true) {
                    log::info!("重建索引 [{}]: 正在处理第 {}/{} 条向量", index_name_clone, i + 1, total_entries);
                    last_log_ts = now;
                }

                let id_str = String::from_utf8_lossy(&key);
                let id: u64 = match id_str.strip_prefix("v:").and_then(|s| s.parse().ok()) {
                    Some(parsed_id) => parsed_id,
                    None => {
                        log::warn!("重建索引: 跳过无效的 key: {:?}", key);
                        continue;
                    }
                };

                // 解析 embedding（根据存储格式）
                let embedding: Vec<f32> = if is_protobuf {
                    match storage_proto::StoredEmbedding::decode(value.as_slice()) {
                        Ok(se) => se.embedding,
                        Err(e) => {
                            log::warn!("重建索引: 解码 protobuf 失败 id={}: {}", id, e);
                            continue;
                        }
                    }
                } else {
                    // 旧格式
                    if value.len() % 4 != 0 {
                        log::warn!(
                            "重建索引: 跳过无效的 value, id={}, value_len={} (不是 4 的倍数)",
                            id,
                            value.len()
                        );
                        continue;
                    }
                    value
                        .chunks_exact(4)
                        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                        .collect()
                };

                // 验证维度
                if embedding.len() != index_dim {
                    log::warn!(
                        "重建索引: 跳过维度不匹配的向量 id={}, expected={}, got={}",
                        id,
                        index_dim,
                        embedding.len()
                    );
                    continue;
                }

                // 检查向量是否包含 NaN 或 Inf
                if embedding.iter().any(|&v| !v.is_finite()) {
                    log::warn!("重建索引: 跳过包含无效值的向量 id={}", id);
                    continue;
                }

                let bf16_vec: Vec<anda_db_hnsw::half::bf16> = embedding
                    .iter()
                    .map(|&v| anda_db_hnsw::half::bf16::from_f32(v))
                    .collect();

                let ts = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                
                match new_index_clone.insert(id, bf16_vec, ts) {
                    Ok(_) => {
                        count += 1;
                    }
                    Err(e) => {
                        log::error!(
                            "重建索引: 插入失败 id={}, key_len={}, value_len={}, embedding_dim={}",
                            id,
                            key.len(),
                            value.len(),
                            embedding.len()
                        );
                        return Err(format!("插入 HNSW 失败 id={}: {}", id, e));
                    }
                }
            }
            log::info!("重建索引 [{}]: 完成处理所有 {} 条向量", index_name_clone, count);
            Ok::<u64, String>(count)
        })
        .await
        .map_err(|e| Status::internal(format!("重建线程异常: {}", e)))?
        .map_err(|e| Status::internal(e))?;

        log::info!("重建索引 [{}]: 插入完成，共 {} 条", index_name, rebuilt_count);

        // 4. 替换旧索引（Arc 应只有一处引用，可直接 unwrap）
        let new_index = Arc::try_unwrap(new_index).unwrap_or_else(|_| {
            // 兜底：clone 一份
            HnswIndex::new(index_name.to_string(), state.config.hnsw_config().ok())
        });
        *state.index.write().await = new_index;

        // 5. 保存快照
        let _ = self.save_snapshot_internal().await;

        log::info!("重建索引 [{}] 完成", index_name);

        Ok(Response::new(RebuildIndexFromRocksDbResponse {
            success: true,
            message: format!("索引重建完成, 重建了 {} 条向量", rebuilt_count),
            rebuilt_count,
        }))
    }
}