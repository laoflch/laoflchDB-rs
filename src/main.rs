use clap::Parser;
use laoflchDB_rust::{cli, db, rpc};
use log::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();

    let cli_args = cli::Cli::parse();

    match cli_args.command {
        cli::Commands::Start { addr, db_path } => {
            info!("=== 启动 laoflchDB gRPC 服务 ===");
            info!("DB 路径: {}", db_path);
            info!("监听地址: {}", addr);
            let db = db::OltpDB::open(&db_path);
            rpc::run_server(&addr, db).await?;
        }
        cli::Commands::Init { db_path } => {
            info!("=== 初始化 laoflchDB ===");
            let _ = std::fs::remove_dir_all(&db_path);
            let mut db = db::OltpDB::open(&db_path);
            db.init_laoflch_db();
            db.print_metadata();
            info!("初始化完成!");
        }
    }

    Ok(())
}
