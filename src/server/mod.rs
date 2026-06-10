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
        
        let access_service = AccessService::with_permissions(
            service.clone(),
            Arc::new(permission_checker.clone()),
        );
        
        Self {
            schema_manager,
            sql_engine,
            service,
            access_service: Arc::new(access_service),
        }
    }

    pub async fn init(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.service.init_database().await?;
        
        info!("LaoflchDBServer 初始化完成");
        Ok(())
    }

    pub async fn start(&self, config: &DatabaseConfig) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.init().await?;

        if config.access_protocols.is_empty() {
            let addr = config.addr.clone();
            
            info!("启动 gRPC 服务: {}", addr);
            println!("\n🚀 LaoflchDB 服务启动成功！");
            println!("   gRPC 服务监听: {}", addr);
            let grpc_service: crate::GrpcService = self.access_service.get_grpc_service(None);
            
            tokio::spawn(async move {
                if let Err(e) = start_grpc_server(grpc_service, &addr).await {
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
                        
                        tokio::spawn(async move {
                            if let Err(e) = start_grpc_server(grpc_service, &addr_owned).await {
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

async fn start_grpc_server<S>(service: S, addr: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> 
where
    S: crate::pb::rpc::laoflch_db_server::LaoflchDb,
{
    use tonic::transport::Server;
    use crate::pb::rpc::laoflch_db_server::LaoflchDbServer;
    
    let addr_copy = addr.to_string();
    info!("gRPC 服务监听: {}", addr_copy);

    Server::builder()
        .add_service(LaoflchDbServer::new(service))
        .serve(addr_copy.parse()?)
        .await?;

    Ok(())
}
