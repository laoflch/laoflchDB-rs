use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "laoflchDB")]
#[command(about = "基于 RocksDB 的 OLTP 数据库 standalone 服务", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    #[command(about = "以 standalone 方式启动 gRPC 服务")]
    Start {
        #[arg(short, long, default_value = "[::1]:50051")]
        addr: String,
        #[arg(short, long, default_value = "./laoflch_db_data")]
        db_path: String,
    },
    Init {
        #[arg(short, long, default_value = "./laoflch_db_data")]
        db_path: String,
    },
}
