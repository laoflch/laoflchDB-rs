use clap::Parser;
use laoflchDB_rust::{cli, config};
use laoflchDB_rust::server::LaoflchDBServer;
use laoflchDB_rust::service::{DatabaseService, DatabaseServiceImpl, SchemaManager};
use laoflchDB_rust::access::AccessService;
use log::info;
use std::sync::Arc;

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
            
            if let Err(e) = start_server(&final_config, &final_db_path) {
                log::error!("服务器启动失败: {}", e);
            }
        }
        cli::Commands::Init { db_path } => {
            let final_db_path = db_path.unwrap_or(config.db_path.clone());
            info!("=== 初始化 laoflchDB ===");
            let _ = std::fs::remove_dir_all(&final_db_path);
            
            let schema_manager = Arc::new(
                SchemaManager::new(&final_db_path)
            );
            let svc: Arc<dyn DatabaseService> = Arc::new(
                DatabaseServiceImpl::new(schema_manager)
            );
            if let Err(e) = svc.init_database() {
                log::error!("初始化数据库失败: {}", e);
                return;
            }
            info!("初始化完成!");
        }
    }
}

fn start_server(
    config: &config::DatabaseConfig,
    db_path: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let schema_manager = Arc::new(
        SchemaManager::new(db_path)
    );
    
    let service_layer: Arc<dyn DatabaseService> = Arc::new(
        DatabaseServiceImpl::new(Arc::clone(&schema_manager))
    );
    
    let access_service = Arc::new(AccessService::new(Arc::clone(&service_layer)));
    
    let server = LaoflchDBServer::new(
        schema_manager,
        service_layer,
        access_service,
    );
    
    server.start(config)?;
    
    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
