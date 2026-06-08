

use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "laoflchdb", version = env!("CARGO_PKG_VERSION"), disable_help_subcommand(true))]
#[command(about = "LaoflchDB 数据库命令行工具")]
pub struct Cli {
    #[arg(short, long, help = "配置文件路径")]
    pub config: Option<String>,
    
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Parser, Debug)]
pub enum Commands {
    #[command(about = "启动数据库服务")]
    Start {
        #[arg(long, help = "监听地址")]
        addr: Option<String>,
        
        #[arg(long, help = "数据库路径")]
        db_path: Option<String>,
    },
    #[command(about = "初始化数据库（幂等操作）")]
    Init {
        #[arg(long, help = "数据库路径")]
        db_path: Option<String>,
        
        #[arg(long, help = "同时初始化示例数据，会删除并重建 example Schema")]
        example: bool,
    },
}
