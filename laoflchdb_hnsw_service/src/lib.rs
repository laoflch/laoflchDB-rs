pub mod proto {
    tonic::include_proto!("laoflchdb.hnsw");
}

use proto::hnsw_index_service_server::HnswIndexService;
use proto::{
    DeleteVectorRequest, DeleteVectorResponse, GetIndexInfoRequest, GetIndexInfoResponse,
    IndexStats, InsertVectorRequest, InsertVectorResponse, SaveSnapshotRequest,
    SaveSnapshotResponse, LoadSnapshotRequest, LoadSnapshotResponse,
    SearchResult, SearchVectorRequest, SearchVectorResponse,
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

/// HNSW 索引配置
#[derive(Debug, Clone)]
pub struct HnswServiceConfig {
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
    /// KV RocksDB 数据存储路径（向量数据持久化）
    pub kv_db_path: String,
    /// HNSW 图拓扑快照保存路径
    pub snapshot_path: String,
}

impl Default for HnswServiceConfig {
    fn default() -> Self {
        Self {
            dim: 512,
            m: 32,
            ef_construction: 200,
            ef_search: 50,
            max_elements: 1_000_000,
            kv_db_path: "./laoflch_hnsw_data".to_string(),
            snapshot_path: "./laoflch_hnsw_snapshots".to_string(),
        }
    }
}

/// HNSW 索引服务实现
///
/// 职责分工：
/// - `anda_db_hnsw` (HnswIndex): 管理内存中的 HNSW 图拓扑（分层可导航小世界图）
/// - `KVRocksDBEngine`: 持久化存储向量数据（RocksDB）
/// - `snapshot_path`: 图拓扑快照文件保存路径（用于启动恢复）
pub struct HnswIndexServiceImpl {
    /// HNSW 内存索引（tokio RwLock 允许跨 .await 持有 guard）
    index: RwLock<HnswIndex>,
    /// 向量数据持久化存储（RocksDB）
    storage: Mutex<KVRocksDBEngine>,
    /// 服务配置
    config: Arc<HnswServiceConfig>,
}

impl HnswIndexServiceImpl {
    /// 创建 HNSW 索引服务实例
    pub async fn new(config: &HnswServiceConfig) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let hnsw_config = HnswConfig {
            dimension: config.dim,
            max_layers: 16,
            max_connections: config.m,
            ef_construction: config.ef_construction,
            ef_search: config.ef_search,
            distance_metric: DistanceMetric::Cosine,
            scale_factor: None,
            select_neighbors_strategy: SelectNeighborsStrategy::Heuristic,
        };

        let index = HnswIndex::new("default".to_string(), Some(hnsw_config));

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

        Ok(Self {
            index: RwLock::new(index),
            storage: Mutex::new(storage),
            config: Arc::new(config.clone()),
        })
    }

    /// 从快照文件恢复 HNSW 图拓扑（如果快照文件存在）
    pub async fn try_load_snapshot(&self) -> Result<Option<u64>, Box<dyn std::error::Error + Send + Sync>> {
        let meta_path = format!("{}/hnsw_index.meta.cbor", self.config.snapshot_path);
        let ids_path = format!("{}/hnsw_index.ids.cbor", self.config.snapshot_path);
        let nodes_path = format!("{}/hnsw_index.nodes.cbor", self.config.snapshot_path);

        // 检查快照文件是否存在（以 meta 文件为准）
        if !std::path::Path::new(&meta_path).exists() {
            log::info!("未找到 HNSW 快照 ({}), 使用空索引启动", meta_path);
            return Ok(None);
        }

        // 读取节点数据到 HashMap
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
        *self.index.write().await = loaded;

        log::info!(
            "HNSW 快照加载成功: {} 条向量, 路径: {}",
            stats.num_elements,
            self.config.snapshot_path
        );
        Ok(Some(stats.num_elements))
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

    /// 保存快照的内部方法：metadata + ids + dirty nodes
    async fn save_snapshot_internal(&self) -> Result<String, Status> {
        tokio::fs::create_dir_all(&self.config.snapshot_path)
            .await
            .map_err(|e| Status::internal(format!("创建快照目录失败: {}", e)))?;

        let stem = format!("{}/hnsw_index", self.config.snapshot_path);
        let meta_path = format!("{}.meta.cbor", stem);
        let ids_path = format!("{}.ids.cbor", stem);
        let nodes_path = format!("{}.nodes.cbor", stem);

        let ts = Self::unix_ms();
        let guard = self.index.read().await;

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

        // 保存 JSON 元数据
        let meta_json = serde_json::json!({
            "dim": self.config.dim,
            "m": self.config.m,
            "ef_construction": self.config.ef_construction,
            "ef_search": self.config.ef_search,
            "saved_at": ts,
        });
        let meta_json_path = format!("{}/hnsw_meta.json", self.config.snapshot_path);
        tokio::fs::write(&meta_json_path, serde_json::to_string_pretty(&meta_json).unwrap())
            .await
            .map_err(|e| Status::internal(format!("保存 JSON 元数据失败: {}", e)))?;

        let snapshot_file = format!("{}/hnsw_index.*", self.config.snapshot_path);
        log::info!("HNSW 快照已保存: {}", snapshot_file);
        Ok(self.config.snapshot_path.clone())
    }

    /// 加载快照的内部方法
    async fn load_snapshot_internal(&self) -> Result<u64, Status> {
        let meta_path = format!("{}/hnsw_index.meta.cbor", self.config.snapshot_path);
        let ids_path = format!("{}/hnsw_index.ids.cbor", self.config.snapshot_path);
        let nodes_path = format!("{}/hnsw_index.nodes.cbor", self.config.snapshot_path);

        // 读取节点数据到 HashMap
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
                    Err(e) => {
                        return Err(Status::internal(format!("解析 nodes 数据失败: {}", e)))
                    }
                }
                cursor
                    .read_exact(&mut buf_len)
                    .map_err(|e| Status::internal(format!("解析 nodes 长度失败: {}", e)))?;
                let data_len = u32::from_le_bytes(buf_len) as usize;
                let mut data = vec![0u8; data_len];
                cursor
                    .read_exact(&mut data)
                    .map_err(|e| Status::internal(format!("解析 nodes 内容失败: {}", e)))?;

                map.insert(u64::from_le_bytes(buf_id), data);
            }

            Arc::new(map)
        };

        let meta_file =
            std::fs::File::open(&meta_path)
                .map_err(|e| Status::not_found(format!("meta 文件不存在: {} ({})", meta_path, e)))?;
        let ids_file =
            std::fs::File::open(&ids_path)
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
        *self.index.write().await = loaded;

        log::info!("HNSW 快照加载成功: {} 条向量", stats.num_elements);
        Ok(stats.num_elements)
    }
}

/// 允许通过 `Arc<HnswIndexServiceImpl>` 直接作为 gRPC 服务注册
#[tonic::async_trait]
impl proto::hnsw_index_service_server::HnswIndexService
    for std::sync::Arc<HnswIndexServiceImpl>
{
    async fn insert_vector(
        &self,
        request: tonic::Request<InsertVectorRequest>,
    ) -> std::result::Result<tonic::Response<InsertVectorResponse>, tonic::Status> {
        self.as_ref().insert_vector(request).await
    }

    async fn search_vector(
        &self,
        request: tonic::Request<SearchVectorRequest>,
    ) -> std::result::Result<tonic::Response<SearchVectorResponse>, tonic::Status> {
        self.as_ref().search_vector(request).await
    }

    async fn delete_vector(
        &self,
        request: tonic::Request<DeleteVectorRequest>,
    ) -> std::result::Result<tonic::Response<DeleteVectorResponse>, tonic::Status> {
        self.as_ref().delete_vector(request).await
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
impl HnswIndexService for HnswIndexServiceImpl {
    /// 插入向量到 HNSW 索引
    async fn insert_vector(
        &self,
        request: Request<InsertVectorRequest>,
    ) -> Result<Response<InsertVectorResponse>, Status> {
        let req = request.into_inner();
        let index_name = if req.index_name.is_empty() {
            "default"
        } else {
            &req.index_name
        };

        // 维度校验
        let dim = self.config.dim;
        if req.embedding.len() != dim {
            return Ok(Response::new(InsertVectorResponse {
                success: false,
                message: format!("向量维度不匹配: 需要 {}, 实际 {}", dim, req.embedding.len()),
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
        let index = self.index.read().await;
        index.insert_f32(req.id, req.embedding, ts).map_err(|e| {
            Status::internal(format!("HNSW 插入失败: {}", e))
        })?;

        log::info!("向量插入成功: id={}, index={}", req.id, index_name);
        Ok(Response::new(InsertVectorResponse {
            success: true,
            message: format!("向量插入成功, id={}", req.id),
        }))
    }

    /// 搜索最近邻向量
    async fn search_vector(
        &self,
        request: Request<SearchVectorRequest>,
    ) -> Result<Response<SearchVectorResponse>, Status> {
        let req = request.into_inner();
        let top_k = if req.top_k <= 0 { 10 } else { req.top_k as usize };

        if req.query_embedding.is_empty() {
            return Ok(Response::new(SearchVectorResponse {
                success: false,
                message: "查询向量为空".to_string(),
                results: vec![],
            }));
        }

        // 执行 HNSW ANN 搜索
        let results = {
            let index = self.index.read().await;
            index.search_f32(&req.query_embedding, top_k).map_err(|e| {
                Status::internal(format!("HNSW 搜索失败: {}", e))
            })?
        };

        if results.is_empty() {
            return Ok(Response::new(SearchVectorResponse {
                success: true,
                message: "未找到匹配结果".to_string(),
                results: vec![],
            }));
        }

        // 从 KV RocksDB 加载向量数据
        let index_name = if req.index_name.is_empty() {
            "default"
        } else {
            &req.index_name
        };
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

        Ok(Response::new(SearchVectorResponse {
            success: true,
            message: format!("搜索完成, 返回 {} 条结果", search_results.len()),
            results: search_results,
        }))
    }

    /// 删除向量
    async fn delete_vector(
        &self,
        request: Request<DeleteVectorRequest>,
    ) -> Result<Response<DeleteVectorResponse>, Status> {
        let req = request.into_inner();
        let index_name = if req.index_name.is_empty() {
            "default"
        } else {
            &req.index_name
        };

        // 1. 从 HNSW 索引删除
        let ts = Self::unix_ms();
        let removed = {
            let index = self.index.read().await;
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
            Ok(Response::new(DeleteVectorResponse {
                success: true,
                message: format!("向量删除成功, id={}", req.id),
            }))
        } else {
            Ok(Response::new(DeleteVectorResponse {
                success: false,
                message: format!("未找到向量 id={}", req.id),
            }))
        }
    }

    /// 获取索引统计信息
    async fn get_index_info(
        &self,
        _request: Request<GetIndexInfoRequest>,
    ) -> Result<Response<GetIndexInfoResponse>, Status> {
        let stats = {
            let index = self.index.read().await;
            index.stats()
        };

        Ok(Response::new(GetIndexInfoResponse {
            success: true,
            message: "ok".to_string(),
            stats: Some(IndexStats {
                num_elements: stats.num_elements,
                max_layers: stats.max_layer as u32,
                // avg_connections 在 anda_db_hnsw v0.4 中已移除
                avg_connections: 0.0,
                search_count: stats.search_count,
                insert_count: stats.insert_count,
                delete_count: stats.delete_count,
                dim: self.config.dim as u32,
                distance_metric: "Cosine".to_string(),
                snapshot_path: self.config.snapshot_path.clone(),
            }),
        }))
    }

    /// 保存 HNSW 图拓扑快照
    async fn save_snapshot(
        &self,
        _request: Request<SaveSnapshotRequest>,
    ) -> Result<Response<SaveSnapshotResponse>, Status> {
        let path = self.save_snapshot_internal().await?;
        Ok(Response::new(SaveSnapshotResponse {
            success: true,
            message: format!("快照已保存: {}", path),
            path,
        }))
    }

    /// 加载 HNSW 图拓扑快照
    async fn load_snapshot(
        &self,
        _request: Request<LoadSnapshotRequest>,
    ) -> Result<Response<LoadSnapshotResponse>, Status> {
        let num_elements = self.load_snapshot_internal().await?;
        Ok(Response::new(LoadSnapshotResponse {
            success: true,
            message: format!("快照加载成功: {} 条向量", num_elements),
            num_elements,
        }))
    }
}