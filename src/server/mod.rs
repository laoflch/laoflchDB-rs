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
        let vector_service = laoflchdb_vector_service::VectorServiceImpl::new_with_config(
            &config.model_path,
            auto_load_models,
        );

        if config.access_protocols.is_empty() {
            let addr = config.addr.clone();
            
            info!("启动 gRPC 服务: {}", addr);
            println!("\n🚀 LaoflchDB 服务启动成功！");
            println!("   gRPC 服务监听: {}", addr);
            let grpc_service: crate::GrpcService = self.access_service.get_grpc_service(None);
            
            tokio::spawn(async move {
                if let Err(e) = start_grpc_server(grpc_service, vector_service, &addr).await {
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
                        let vector_service_clone = laoflchdb_vector_service::VectorServiceImpl::new_with_config(
                            &config.model_path,
                            auto_load_models.clone(),
                        );
                        
                        tokio::spawn(async move {
                            if let Err(e) = start_grpc_server(grpc_service, vector_service_clone, &addr_owned).await {
                                log::error!("gRPC 服务错误: {}", e);
                            }
                        });
                    }
                    "rest" | "http" => {
                        info!("启动 REST 服务: {} (service_id: {:?})", addr, service_id);
                        started_protocols.push((protocol.to_string(), addr.to_string()));
                        let rest_service = self.access_service.get_rest_service(service_id);
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
    addr: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use tonic::transport::Server;
    use crate::pb::rpc::laoflch_db_server::LaoflchDbServer;
    use laoflchdb_vector_service::proto::vector_service_server::VectorServiceServer;
    
    let addr_copy = addr.to_string();
    info!("gRPC 服务监听: {}", addr_copy);

    Server::builder()
        .add_service(LaoflchDbServer::new(laoflchdb_service))
        .add_service(VectorServiceServer::new(vector_service))
        .serve(addr_copy.parse()?)
        .await?;

    Ok(())
}