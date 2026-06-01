

use clap::Parser;
use laoflchDB_rust::{cli, config, init_data};
use laoflchDB_rust::server::LaoflchDBServer;
use laoflchDB_rust::service::{DatabaseService, DatabaseServiceImpl, SchemaManager};
use laoflchDB_rust::access::AccessService;
use log::info;
use std::sync::Arc;
use std::path::Path;

#[tokio::main]
async fn main() {
    let cli_args = cli::Cli::parse();

    let config = match cli_args.config {
        Some(path) => {
            info!("加载配置文件: {}", path);
            config::DatabaseConfig::load_from_file(path).expect("加载配置文件失败")
        }
        None => config::DatabaseConfig::load_or_default(),
    };

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(&config.log_level))
        .format_timestamp_millis()
        .init();

    match cli_args.command {
        cli::Commands::Start { addr, db_path } => {
            let final_db_path = db_path.unwrap_or(config.db_path.clone());
            let mut final_config = config.clone();
            if addr.is_some() {
                final_config.addr = addr.unwrap();
            }
            
            info!("=== 启动 laoflchDB 服务 ===");
            info!("DB 路径: {}", final_db_path);
            info!("监听地址: {}", final_config.addr);
            
            if let Err(e) = start_server(&final_config, &final_db_path).await {
                log::error!("服务器启动失败: {}", e);
            }
        }
        cli::Commands::Init { db_path, example } => {
            let final_db_path = db_path.unwrap_or(config.db_path.clone());
            info!("=== 初始化 laoflchDB ===");
            
            let db_path_exists = Path::new(&final_db_path).exists();
            
            if db_path_exists {
                info!("数据库目录已存在，跳过删除步骤");
            } else {
                info!("数据库目录不存在，创建新目录");
            }
            
            let svc: Arc<dyn DatabaseService> = Arc::new(
                DatabaseServiceImpl::new(&final_db_path).await
            );
            
            if !db_path_exists {
                if let Err(e) = svc.init_database().await {
                    log::error!("初始化数据库失败: {}", e);
                    return;
                }
                info!("初始化完成!");
            } else {
                info!("数据库已初始化，跳过初始化步骤");
            }
            
            if example {
                if let Err(e) = init_data::init_example_data(&final_db_path).await {
                    log::error!("初始化示例数据失败: {}", e);
                    return;
                }
            }
        }
    }
}

async fn start_server(
    config: &config::DatabaseConfig,
    db_path: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let service_layer: Arc<dyn DatabaseService> = Arc::new(
        DatabaseServiceImpl::new(db_path).await
    );
    
    let server = LaoflchDBServer::new(
        Arc::new(SchemaManager::new(db_path).await),
        service_layer,
        Arc::new(AccessService::new(Arc::new(DatabaseServiceImpl::new(db_path).await))),
        config,
    ).await;
    
    server.start(config).await?;
    
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}

