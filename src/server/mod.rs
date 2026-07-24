use crate::service::SchemaManager;
use crate::service::DatabaseService;
use crate::access::{AccessService, PermissionChecker};
use crate::config::DatabaseConfig;
use laoflchdb_engines::SQLEngine;
use std::sync::Arc;
use log::info;

pub struct LaoflchDBServer {
    schema_manager: Arc<SchemaManager>,
    sql_engine: Arc<tokio::sync::RwLock<dyn SQLEngine>>,
    service: Arc<dyn DatabaseService>,
    access_service: Arc<AccessService>,
}

impl LaoflchDBServer {
    pub async fn new(
        schema_manager: Arc<SchemaManager>,
        sql_engine: Arc<tokio::sync::RwLock<dyn SQLEngine>>,
        service: Arc<dyn DatabaseService>,
        _access_service: Arc<AccessService>,
        config: &DatabaseConfig,
    ) -> Self {
        let global_default = config.get_global_default_policy();
        let mut permission_checker = PermissionChecker::new(global_default);
        
        if let Some(ref permissions) = config.permissions {
            for perm in permissions {
                permission_checker.add_service_permission(perm.clone());
            }
        }
        
        let access_service = _access_service;
        
        Self {
            schema_manager,
            sql_engine,
            service,
            access_service,
        }
    }

    pub async fn init(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.service.init_database().await?;
        
        info!("LaoflchDBServer 初始化完成");
        Ok(())
    }

    pub async fn start(&self, config: &DatabaseConfig) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.init().await?;

        // 创建向量化服务实例（从配置的模型目录自动加载模型）
        let auto_load_models = config.vector_service.as_ref().map(|vc| {
            if vc.auto_load {
                if vc.load_models.is_empty() {
                    None // 加载 candle 下所有
                } else {
                    Some(vc.load_models.clone()) // 加载指定的
                }
            } else {
                Some(vec![]) // 不加载任何
            }
        }).flatten();

        // 创建嵌入向量索引服务（如果配置启用）
        let embedding_service = match &config.embedding_index {
            Some(embedding_cfg) if embedding_cfg.enabled => {
                let indices: Vec<laoflchdb_embedding_service::IndexConfig> = embedding_cfg.indices.iter().map(|idx| {
                    laoflchdb_embedding_service::IndexConfig {
                        name: idx.name.clone(),
                        dim: idx.dim,
                        m: idx.m as u8,
                        ef_construction: idx.ef_construction as usize,
                        ef_search: idx.ef_search as usize,
                        max_elements: idx.max_elements as u64,
                        distance_metric: laoflchdb_embedding_service::IndexConfig::distance_metric_from_str(&idx.distance_metric),
                    }
                }).collect();

                let embedding_config = laoflchdb_embedding_service::EmbeddingServiceConfig {
                    indices,
                    kv_db_path: embedding_cfg.kv_db_path.clone(),
                    snapshot_path: embedding_cfg.snapshot_path.clone(),
                };
                match laoflchdb_embedding_service::EmbeddingIndexServiceImpl::new(&embedding_config).await {
                    Ok(svc) => {
                        // 尝试自动加载快照
                        if let Ok(Some(n)) = svc.try_load_snapshot().await {
                            info!("嵌入向量索引从快照恢复: {} 条向量", n);
                        }
                        info!("嵌入向量索引服务已启动");
                        Some(Arc::new(svc))
                    }
                    Err(e) => {
                        log::error!("嵌入向量索引服务启动失败: {}", e);
                        None
                    }
                }
            }
            _ => {
                info!("嵌入向量索引服务未启用");
                None
            }
        };

        // 创建向量化服务实例（在图片服务之前创建，因为图片服务需要引用）
        let vector_service = laoflchdb_vector_service::VectorServiceImpl::new_with_config(
            &config.model_path,
            auto_load_models.clone(),
        );

        // 创建对象存储服务（如果配置启用）
        let object_store_service = match &config.object_store {
            Some(obj_cfg) if obj_cfg.enabled => {
                let obj_config = laoflchdb_object_store_service::ObjectStoreConfig {
                    enabled: true,
                    db_path: obj_cfg.db_path.clone(),
                    schema_name: "object_store".to_string(),
                    blob_db: laoflchdb_kv_rocksdb_engine::BlobDBConfig::default(),
                };
                match laoflchdb_object_store_service::ObjectStoreServiceImpl::new(&obj_config).await {
                    Ok(svc) => {
                        info!("对象存储服务已启动");
                        Some(Arc::new(svc))
                    }
                    Err(e) => {
                        log::error!("对象存储服务启动失败: {}", e);
                        None
                    }
                }
            }
            _ => {
                info!("对象存储服务未启用");
                None
            }
        };

        // 创建图片服务（如果配置启用，且对象存储服务已启动）
        let image_service = match (&config.image_service, &object_store_service) {
            (Some(img_cfg), Some(os_svc)) if img_cfg.enabled => {
                let img_config = laoflchdb_image_service::ImageServiceConfig {
                    enabled: true,
                    default_bucket: img_cfg.default_bucket.clone(),
                };
                let img_svc = laoflchdb_image_service::ImageServiceImpl::new(
                    os_svc.clone(),
                    img_config,
                    vector_service.clone(),
                    embedding_service.clone(),
                );
                info!("图片服务已启动");
                Some(Arc::new(img_svc))
            }
            (Some(img_cfg), _) if img_cfg.enabled => {
                log::warn!("图片服务已启用但对象存储服务未启用，图片服务需要对象存储服务支持，将不启动图片服务");
                None
            }
            _ => {
                info!("图片服务未启用");
                None
            }
        };

        // 创建人脸服务（如果配置启用）
        let face_service = match (&config.face_service, &image_service) {
            (Some(face_cfg), Some(img_svc)) if face_cfg.enabled => {
                let face_config = laoflchdb_face_service::FaceServiceConfig {
                    enabled: true,
                    model_dir: face_cfg.model_dir.clone(),
                    scrfd_model_file: face_cfg.scrfd_model_file.clone(),
                    arcface_model_file: face_cfg.arcface_model_file.clone(),
                    det_threshold: face_cfg.det_threshold,
                    max_faces: face_cfg.max_faces,
                };
                let face_svc = laoflchdb_face_service::FaceServiceImpl::new(
                    face_config,
                    Some(img_svc.clone()),
                    embedding_service.clone(),
                );
                info!("人脸服务已启动");
                Some(Arc::new(face_svc))
            }
            (Some(face_cfg), _) if face_cfg.enabled => {
                log::warn!("人脸服务已启用但图片服务未启用，人脸服务需要图片服务支持保存对齐人脸图片，将不启动人脸服务");
                None
            }
            _ => {
                info!("人脸服务未启用");
                None
            }
        };

            let object_store_service = object_store_service.clone();

        if config.access_protocols.is_empty() {
            let addr = config.addr.clone();

            info!("启动 gRPC 服务: {}", addr);
            println!("\n🚀 LaoflchDB 服务启动成功！");
            println!("   gRPC 服务监听: {}", addr);
            let grpc_service: crate::GrpcService = self.access_service.get_grpc_service(None);

            let vector_service = vector_service.clone();
            tokio::spawn(async move {
                if let Err(e) = start_grpc_server(grpc_service, vector_service, embedding_service, object_store_service, image_service, face_service, &addr).await {
                    log::error!("gRPC 服务错误: {}", e);
                }
            });
        } else {
            let mut started_protocols = Vec::new();
            
            for protocol_config in &config.access_protocols {
                if !protocol_config.enabled {
                    continue;
                }

                let addr = protocol_config.addr.as_ref().unwrap_or(&config.addr);
                let protocol = &protocol_config.protocol;
                let service_id = protocol_config.service_id.clone();

                match protocol.as_str() {
                    "grpc" => {
                        info!("启动 gRPC 服务: {} (service_id: {:?})", addr, service_id);
                        started_protocols.push((protocol.to_string(), addr.to_string()));
                        let grpc_service = self.access_service.get_grpc_service(service_id);
                        let addr_owned = addr.to_string();
                        let vector_service_clone = vector_service.clone();
                        let embedding_service_clone = embedding_service.clone();

                        let object_store_service_clone = object_store_service.clone();
                        let image_service_clone = image_service.clone();
                        let face_service_clone = face_service.clone();

                        tokio::spawn(async move {
                            if let Err(e) = start_grpc_server(grpc_service, vector_service_clone, embedding_service_clone, object_store_service_clone, image_service_clone, face_service_clone, &addr_owned).await {
                                log::error!("gRPC 服务错误: {}", e);
                            }
                        });
                    }
                    "rest" | "http" => {
                        info!("启动 REST 服务: {} (service_id: {:?})", addr, service_id);
                        started_protocols.push((protocol.to_string(), addr.to_string()));
                        let mut rest_service = self.access_service.get_rest_service(service_id);
                        // 如果对象存储服务已启用，创建并挂载其 REST 路由
                        if let Some(ref os_svc) = object_store_service {
                            let os_router = laoflchdb_object_store_service::create_rest_router(os_svc.clone());
                            rest_service = rest_service.with_object_store_router(os_router);
                        }
                        // 如果图片服务已启用，创建并挂载其 REST 路由
                        if let Some(ref img_svc) = image_service {
                            let img_router = laoflchdb_image_service::create_rest_router(img_svc.clone());
                            rest_service = rest_service.with_image_router(img_router);
                        }
                        // 如果人脸服务已启用，创建并挂载其 REST 路由
                        if let Some(ref face_svc) = face_service {
                            let face_router = laoflchdb_face_service::create_rest_router(face_svc.clone());
                            rest_service = rest_service.with_face_router(face_router);
                        }
                        let addr_owned = addr.to_string();

                        tokio::spawn(async move {
                            if let Err(e) = rest_service.start(&addr_owned).await {
                                log::error!("REST 服务启动失败: {}", e);
                            }
                        });
                    }
                    other => {
                        log::warn!("不支持的协议类型: {}", other);
                    }
                }
            }
            
            println!("\n🚀 LaoflchDB 服务启动成功！");
            for (protocol, addr) in started_protocols {
                println!("   {} 服务监听: {}", protocol.to_uppercase(), addr);
            }
        }

        Ok(())
    }

    pub fn schema_manager(&self) -> &Arc<SchemaManager> {
        &self.schema_manager
    }
    
    pub fn sql_engine(&self) -> &Arc<tokio::sync::RwLock<dyn SQLEngine>> {
        &self.sql_engine
    }
}

async fn start_grpc_server(
    laoflchdb_service: impl crate::pb::rpc::laoflch_db_server::LaoflchDb,
    vector_service: impl laoflchdb_vector_service::proto::vector_service_server::VectorService,
    embedding_service: Option<std::sync::Arc<laoflchdb_embedding_service::EmbeddingIndexServiceImpl>>,
    object_store_service: Option<std::sync::Arc<laoflchdb_object_store_service::ObjectStoreServiceImpl>>,
    image_service: Option<std::sync::Arc<laoflchdb_image_service::ImageServiceImpl>>,
    face_service: Option<std::sync::Arc<laoflchdb_face_service::FaceServiceImpl>>,
    addr: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use tonic::transport::Server;
    use crate::pb::rpc::laoflch_db_server::LaoflchDbServer;
    use laoflchdb_vector_service::proto::vector_service_server::VectorServiceServer;
    use laoflchdb_embedding_service::proto::embedding_index_service_server::EmbeddingIndexServiceServer;
    use laoflchdb_object_store_service::proto::object_store_service_server::ObjectStoreServiceServer;
    use laoflchdb_image_service::proto::image_service_server::ImageServiceServer;
    use laoflchdb_face_service::proto::face_service_server::FaceServiceServer;

    let addr_copy = addr.to_string();
    info!("gRPC 服务监听: {}", addr_copy);

    let max_frame_size: u32 = 5 * 1024 * 1024; // 5MB（客户端切片为 4MB，留出 protobuf 编码开销空间）
    let mut server = Server::builder()
        .max_frame_size(Some(max_frame_size))
        .add_service(LaoflchDbServer::new(laoflchdb_service))
        .add_service(VectorServiceServer::new(vector_service));

    // 如果有嵌入向量索引服务配置，则注册
    if let Some(embedding) = embedding_service {
        server = server.add_service(EmbeddingIndexServiceServer::new(embedding));
    }

    // 如果有对象存储服务配置，则注册
    if let Some(object_store) = object_store_service {
        server = server.add_service(ObjectStoreServiceServer::new(object_store));
    }

    // 如果有图片服务配置，则注册
    if let Some(image) = image_service {
        server = server.add_service(ImageServiceServer::new(image));
    }

    // 如果有 face 服务配置，则注册
    if let Some(face) = face_service {
        server = server.add_service(FaceServiceServer::from_arc(face));
    }

    server.serve(addr_copy.parse()?).await?;

    Ok(())
}