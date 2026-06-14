use laoflchDB_rust::{
    Cli, Commands, DatabaseConfig, LaoflchDBServer, RuntimeMode,
    AccessService, init_data, engine_factory, DatabaseService,
    IndexService, IndexServiceImpl,
};
use laoflchDB_rust::access::permission::PermissionChecker;
use clap::Parser;
use std::sync::Arc;
use log::{info, warn};
use tokio::runtime::{Builder, Runtime};
use tokio::signal;

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    env_logger::init();
    
    let cli = Cli::parse();
    let config = if let Some(ref config_path) = cli.config {
        DatabaseConfig::load_from_file(config_path)?
    } else {
        DatabaseConfig::load_or_default()
    };
    
    let runtime = match config.runtime_mode {
        RuntimeMode::MultiThread => {
            info!("使用多线程运行时");
            Builder::new_multi_thread().enable_all().build()?
        }
        RuntimeMode::SingleThread => {
            info!("使用单线程运行时");
            Builder::new_current_thread().enable_all().build()?
        }
    };
    
    runtime.block_on(async move {
        match cli.command {
            Commands::Start { addr, db_path,index_path } => {
                start_server(&config, addr.as_deref(), db_path.as_deref(),index_path.as_deref()).await
            }
            Commands::Init { db_path, example } => {
                init_database(&config, db_path.as_deref(), example).await
            }
        }
    })
}

async fn start_server(
    config: &DatabaseConfig,
    addr: Option<&str>,
    db_path: Option<&str>,
    index_path: Option<&str>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let effective_db_path = db_path.unwrap_or(&config.db_path);
    let effective_index_path = index_path.unwrap_or(&config.index_path);
    let effective_addr = addr.unwrap_or(&config.addr);
    
    info!("启动 LaoflchDB 服务...");
    info!("数据库路径: {}", effective_db_path);
    info!("监听地址: {}", effective_addr);
    
    let service = engine_factory::create_default_database_service(effective_db_path).await?;
    let service_clone = service.clone();
    let sql_engine = service.sql_engine().clone();
    info!("IndexService initialized at path: {}", effective_index_path);
    let index_path = format!("{}/indexes", effective_index_path);
    info!("尝试初始化全文索引服务，路径: {}", index_path);
    
    let access_service = match IndexServiceImpl::new(&index_path, "fulltext").await {
        Ok(index_service) => {
            info!("全文索引服务已成功初始化");
            let permission_checker = Arc::new(PermissionChecker::new(true));
            Arc::new(AccessService::with_permissions_and_index(
                service.clone(),
                permission_checker,
                Arc::new(index_service)
            ))
        },
        Err(e) => {
            log::error!("全文索引服务初始化失败，路径: {}, 错误: {}", index_path, e);
            log::warn!("将不启用索引功能，REST API 的索引端点将不可用");
            Arc::new(AccessService::new(service.clone()))
        }
    };
    
    let server = LaoflchDBServer::new(
        service.schema_manager().clone(),
        sql_engine,
        service,
        access_service,
        config,
    ).await;
    
    server.start(config).await?;
    
    let mut sigint = signal::unix::signal(signal::unix::SignalKind::interrupt())?;
    let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate())?;
    
    tokio::select! {
        _ = sigint.recv() => {
            info!("收到 SIGINT 信号 (Ctrl+C)");
        },
        _ = sigterm.recv() => {
            info!("收到 SIGTERM 信号");
        },
        _ = tokio::signal::ctrl_c() => {
            info!("收到 Ctrl+C");
        }
    }
    
    info!("正在关闭数据库服务...");
    match service_clone.shutdown().await {
        Ok(_) => info!("数据库服务关闭成功"),
        Err(e) => warn!("关闭数据库服务时出错: {}", e),
    }
    
    Ok(())
}

async fn init_database(
    config: &DatabaseConfig,
    db_path: Option<&str>,
    example: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let effective_db_path = db_path.unwrap_or(&config.db_path);
    
    info!("初始化数据库...");
    info!("数据库路径: {}", effective_db_path);
    
    {
        let service = engine_factory::create_default_database_service(effective_db_path).await?;
        service.init_database().await?;
        
        if example {
            info!("初始化示例数据...");
            init_data::init_example_data(&service).await?;
        }
    }
    
    info!("数据库初始化完成");
    Ok(())
}
