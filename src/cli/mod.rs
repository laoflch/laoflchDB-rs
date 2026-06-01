

use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "laoflchdb")]
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
    #[command(about = "初始化数据库")]
    Init {
        #[arg(long, help = "数据库路径")]
        db_path: Option<String>,
        
        #[arg(long, help = "同时初始化示例数据")]
        example: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parse_default_config() {
        let args = vec!["laoflchdb", "start"];
        let cli = Cli::try_parse_from(args).unwrap();
        assert!(cli.config.is_none());
        match cli.command {
            Commands::Start { addr, db_path } => {
                assert!(addr.is_none());
                assert!(db_path.is_none());
            }
            _ => panic!("Expected Start command"),
        }
    }

    #[test]
    fn test_cli_parse_with_config() {
        let args = vec!["laoflchdb", "-c", "config.yaml", "init"];
        let cli = Cli::try_parse_from(args).unwrap();
        assert_eq!(cli.config, Some("config.yaml".to_string()));
        match cli.command {
            Commands::Init { db_path, example } => {
                assert!(db_path.is_none());
                assert!(!example);
            }
            _ => panic!("Expected Init command"),
        }
    }

    #[test]
    fn test_cli_parse_start_with_options() {
        let args = vec!["laoflchdb", "start", "--addr", "0.0.0.0:9090", "--db-path", "/tmp/db"];
        let cli = Cli::try_parse_from(args).unwrap();
        match cli.command {
            Commands::Start { addr, db_path } => {
                assert_eq!(addr, Some("0.0.0.0:9090".to_string()));
                assert_eq!(db_path, Some("/tmp/db".to_string()));
            }
            _ => panic!("Expected Start command"),
        }
    }
    
    #[test]
    fn test_cli_parse_init_with_example() {
        let args = vec!["laoflchdb", "init", "--example"];
        let cli = Cli::try_parse_from(args).unwrap();
        match cli.command {
            Commands::Init { db_path, example } => {
                assert!(db_path.is_none());
                assert!(example);
            }
            _ => panic!("Expected Init command"),
        }
    }
}

