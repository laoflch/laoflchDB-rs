use clap::Parser;
use serde::{Deserialize, Serialize};
use std::process::Command;
use std::path::Path;

const GCC13_BIN: &str = "/usr/local/gcc-13/bin";
const ROCKSDB_REGISTRY_PATH: &str = "/home/laoflch/.cargo/registry/src/mirrors.ustc.edu.cn-38d0e5eb5da2abae/rust-librocksdb-sys-0.46.0+11.1.1/rocksdb";
const CONFIG_PATH_LOCAL: &str = "laoflchdb.yaml";
const CONFIG_PATH_PROD: &str = "config/prod.yaml";

#[derive(Debug, Deserialize, Serialize)]
struct Config {
    access_protocols: Vec<AccessProtocol>,
}

#[derive(Debug, Deserialize, Serialize)]
struct AccessProtocol {
    protocol: String,
    addr: String,
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
enum Cli {
    Build,
    #[command(subcommand)]
    Docker(DockerCommand),
    #[command(subcommand)]
    AutoTest(AutoTestCommand),
    Ldb,
    All,
    Init,
}

#[derive(Parser, Debug)]
enum DockerCommand {
    Build,
    Deploy,
    Start,
}

#[derive(Parser, Debug)]
enum AutoTestCommand {
    Local,
    Prod,
}

fn main() {
    let args = Cli::parse();

    match args {
        Cli::Build => build_project(),
        Cli::Docker(docker_cmd) => match docker_cmd {
            DockerCommand::Build => build_docker(),
            DockerCommand::Deploy => deploy(),
            DockerCommand::Start => start_docker(),
        },
        Cli::AutoTest(auto_test_cmd) => match auto_test_cmd {
            AutoTestCommand::Local => run_auto_test_local(),
            AutoTestCommand::Prod => run_auto_test_prod(),
        },
        Cli::Ldb => build_ldb(),
        Cli::All => build_all(),
        Cli::Init => init_database(),
    }
}

fn build_project() {
    println!("Building project...");
    let status = Command::new("cargo")
        .args(["build", "--release", "--features=production"])
        .status()
        .expect("Failed to build project");
    
    if status.success() {
        println!("Project built successfully");
    } else {
        std::process::exit(1);
    }
}

fn build_docker() {
    println!("Building Docker image...");
    let status = Command::new("docker")
        .args(["build", "-f", "Dockerfile.prod", "-t", "laoflchdb-rust:prod", "."])
        .status()
        .expect("Failed to build Docker image");
    
    if status.success() {
        println!("Docker image built successfully");
    } else {
        std::process::exit(1);
    }
}

fn get_ports_from_config(config_path: &str) -> (String, String) {
    let config_content = std::fs::read_to_string(config_path)
        .expect(&format!("Failed to read config file: {}", config_path));
    
    let config: Config = serde_yaml::from_str(&config_content)
        .expect("Failed to parse config file");
    
    let mut grpc_port = "29777".to_string();
    let mut rest_port = "38080".to_string();
    
    for protocol in &config.access_protocols {
        let addr_parts: Vec<&str> = protocol.addr.split(':').collect();
        if addr_parts.len() == 2 {
            let port = addr_parts[1].to_string();
            match protocol.protocol.as_str() {
                "grpc" => grpc_port = port,
                "rest" => rest_port = port,
                _ => {}
            }
        }
    }
    
    (grpc_port, rest_port)
}

fn start_docker() {
    println!("Starting Docker container...");
    
    let (grpc_port, rest_port) = get_ports_from_config(CONFIG_PATH_PROD);
    println!("Using ports - gRPC: {}, REST: {}", grpc_port, rest_port);
    
    let status = Command::new("docker")
        .args(["run", "-d", "--name", "laoflchdb", "--privileged",
               "-p", &format!("{}:{}", grpc_port, grpc_port),
               "-p", &format!("{}:{}", rest_port, rest_port),
               "-v", "/workspace/rust_space/laoflchDB-rust/laoflch_db_data_prod:/app/data",
               "laoflchdb-rust:prod"])
        .status()
        .expect("Failed to start Docker container");
    
    if status.success() {
        println!("Docker container started successfully");
    } else {
        std::process::exit(1);
    }
}

fn deploy() {
    println!("=== Starting deployment ===");
    
    build_project();
    build_docker();
    start_docker();
    
    println!("=== Deployment completed ===");
}

fn run_auto_test_local() {
    println!("{}", "=".repeat(60));
    println!("Running Python auto tests for LOCAL environment...");
    println!("{}", "=".repeat(60));
    
    let (_, rest_port) = get_ports_from_config(CONFIG_PATH_LOCAL);
    println!("Using REST port: {}", rest_port);
    
    std::env::set_var("LAOFLCHDB_REST_PORT", &rest_port);
    
    let status = Command::new("python3")
        .args(["tests_python/test_e2e_rest.py"])
        .status()
        .expect("Failed to run REST tests");
    
    if !status.success() {
        eprintln!("❌ REST tests failed");
        std::process::exit(1);
    }
    println!("✅ REST tests passed");
    
    let status = Command::new("python3")
        .args(["tests_python/test_final.py"])
        .status()
        .expect("Failed to run gRPC tests");
    
    if !status.success() {
        eprintln!("❌ gRPC tests failed");
        std::process::exit(1);
    }
    println!("✅ gRPC tests passed");
    
    println!();
    println!("{}", "=".repeat(60));
    println!("✅ All local tests passed!");
    println!("{}", "=".repeat(60));
}

fn run_auto_test_prod() {
    println!("{}", "=".repeat(60));
    println!("Running Python auto tests for PROD environment...");
    println!("{}", "=".repeat(60));
    
    let (_, rest_port) = get_ports_from_config(CONFIG_PATH_PROD);
    println!("Using REST port: {}", rest_port);
    
    std::env::set_var("LAOFLCHDB_REST_PORT", &rest_port);
    
    let status = Command::new("python3")
        .args(["tests_python/test_e2e_rest.py"])
        .status()
        .expect("Failed to run REST tests");
    
    if !status.success() {
        eprintln!("❌ REST tests failed");
        std::process::exit(1);
    }
    println!("✅ REST tests passed");
    
    let status = Command::new("python3")
        .args(["tests_python/test_final.py"])
        .status()
        .expect("Failed to run gRPC tests");
    
    if !status.success() {
        eprintln!("❌ gRPC tests failed");
        std::process::exit(1);
    }
    println!("✅ gRPC tests passed");
    
    println!();
    println!("{}", "=".repeat(60));
    println!("✅ All prod tests passed!");
    println!("{}", "=".repeat(60));
}

fn build_ldb() {
    println!("{}", "=".repeat(60));
    println!("构建 RocksDB ldb 工具...");
    println!("源码路径: {}", ROCKSDB_REGISTRY_PATH);
    println!("{}", "=".repeat(60));
    
    let rocksdb_path = Path::new(ROCKSDB_REGISTRY_PATH);
    if !rocksdb_path.exists() {
        eprintln!("❌ RocksDB 源码不存在: {}", ROCKSDB_REGISTRY_PATH);
        eprintln!("请先运行 cargo fetch 下载依赖");
        std::process::exit(1);
    }
    
    let mut env = std::env::vars().collect::<Vec<_>>();
    env.push(("PATH".to_string(), format!("{}:{}", GCC13_BIN, std::env::var("PATH").unwrap_or_default())));
    env.push(("CC".to_string(), format!("{}/gcc", GCC13_BIN)));
    env.push(("CXX".to_string(), format!("{}/g++", GCC13_BIN)));
    
    let mut cmd = Command::new("make");
    cmd.args(&["ldb", "-j8", "DEBUG_LEVEL=0"])
       .current_dir(ROCKSDB_REGISTRY_PATH);
    
    for (key, value) in env {
        cmd.env(&key, &value);
    }
    
    let status = cmd.status().expect("Failed to run make");
    
    if !status.success() {
        eprintln!("❌ ldb 工具构建失败");
        std::process::exit(1);
    }
    
    let ldb_source = Path::new(ROCKSDB_REGISTRY_PATH).join("ldb");
    let ldb_target = Path::new("target").join("release").join("ldb");
    
    if !ldb_source.exists() {
        eprintln!("❌ ldb 源文件不存在");
        std::process::exit(1);
    }
    
    if let Some(parent) = ldb_target.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    
    std::fs::copy(&ldb_source, &ldb_target).expect("Failed to copy ldb");
    println!("✅ ldb 已复制到 {}", ldb_target.display());
    println!("✅ ldb 工具构建成功");
}

fn build_all() {
    println!("{}", "=".repeat(60));
    println!("构建全部 (Rust + ldb)...");
    println!("{}", "=".repeat(60));
    
    build_project();
    println!();
    build_ldb();
    
    println!();
    println!("{}", "=".repeat(60));
    println!("✅ 所有构建完成");
    println!("{}", "=".repeat(60));
}

fn init_database() {
    println!("{}", "=".repeat(60));
    println!("初始化数据库...");
    println!("{}", "=".repeat(60));
    
    let status = Command::new("./target/release/laoflchDB-rust")
        .args(["init", "--example"])
        .status()
        .expect("Failed to run init command");
    
    if status.success() {
        println!("✅ 数据库初始化成功");
    } else {
        eprintln!("❌ 数据库初始化失败");
        std::process::exit(1);
    }
}