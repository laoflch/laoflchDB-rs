use crate::service::SchemaManager;
use crate::service::DatabaseService;
use crate::access::AccessService;
use crate::config::DatabaseConfig;
use std::sync::Arc;
use log::info;

pub struct LaoflchDBServer {
    #[allow(dead_code)]
    schema_manager: Arc<SchemaManager>,
    service: Arc<dyn DatabaseService>,
    access_service: Arc<AccessService>,
}

impl LaoflchDBServer {
    pub fn new(
        schema_manager: Arc<SchemaManager>,
        service: Arc<dyn DatabaseService>,
        access_service: Arc<AccessService>,
    ) -> Self {
        Self {
            schema_manager,
            service,
            access_service,
        }
    }

    pub fn init(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.service.init_database()?;
        info!("LaoflchDBServer 初始化完成");
        Ok(())
    }

    pub fn start(&self, config: &DatabaseConfig) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.init()?;

        if config.access_protocols.is_empty() {
            let addr = config.addr.clone();
            
            info!("启动 gRPC 服务: {}", addr);
            let grpc_service = self.access_service.get_grpc_service();
            
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(start_grpc_server(grpc_service, &addr)).unwrap();
            });
        } else {
            for protocol_config in &config.access_protocols {
                if !protocol_config.enabled {
                    continue;
                }

                let addr = protocol_config.addr.as_ref().unwrap_or(&config.addr);
                let protocol = &protocol_config.protocol;

                match protocol.as_str() {
                    "grpc" => {
                        info!("启动 gRPC 服务: {}", addr);
                        let grpc_service = self.access_service.get_grpc_service();
                        let addr_owned = addr.to_string();
                        
                        std::thread::spawn(move || {
                            let rt = tokio::runtime::Runtime::new().unwrap();
                            rt.block_on(start_grpc_server(grpc_service, &addr_owned)).unwrap();
                        });
                    }
                    other => {
                        log::warn!("不支持的协议类型: {}", other);
                    }
                }
            }
        }

        Ok(())
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
