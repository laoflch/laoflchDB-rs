use laoflchDB_rust::{Cli, Commands};
use clap::Parser;

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

#[test]
fn test_cli_parse_init_with_db_path() {
    let args = vec!["laoflchdb", "init", "--db-path", "/data/laoflchdb"];
    let cli = Cli::try_parse_from(args).unwrap();
    match cli.command {
        Commands::Init { db_path, example } => {
            assert_eq!(db_path, Some("/data/laoflchdb".to_string()));
            assert!(!example);
        }
        _ => panic!("Expected Init command"),
    }
}

#[test]
fn test_cli_parse_init_full_options() {
    let args = vec!["laoflchdb", "-c", "config.yaml", "init", "--db-path", "/data/laoflchdb", "--example"];
    let cli = Cli::try_parse_from(args).unwrap();
    assert_eq!(cli.config, Some("config.yaml".to_string()));
    match cli.command {
        Commands::Init { db_path, example } => {
            assert_eq!(db_path, Some("/data/laoflchdb".to_string()));
            assert!(example);
        }
        _ => panic!("Expected Init command"),
    }
}