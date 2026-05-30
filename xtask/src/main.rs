use std::process::Command;
use std::path::Path;

const GCC13_BIN: &str = "/usr/local/gcc-13/bin";
const ROCKSDB_REGISTRY_PATH: &str = "/home/laoflch/.cargo/registry/src/mirrors.ustc.edu.cn-38d0e5eb5da2abae/rust-librocksdb-sys-0.46.0+11.1.1/rocksdb";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    
    if args.len() < 2 {
        build_rust();
        return;
    }
    
    match args[1].as_str() {
        "build" => {
            if args.len() > 2 && args[2] == "--release" {
                build_rust_release();
            } else {
                build_rust();
            }
        }
        "ldb" => {
            build_ldb();
        }
        "all" => {
            build_all();
        }
        _ => {
            eprintln!("未知命令: {}", args[1]);
            print_usage();
            std::process::exit(1);
        }
    }
}

fn print_usage() {
    println!("使用方法:");
    println!("  cargo ldb        # 构建ldb工具");
    println!("  cargo all        # 构建所有 (Rust + ldb)");
    println!();
    println!("或者在项目根目录运行:");
    println!("  cargo build      # 默认构建Rust源码");
}

fn build_rust() {
    println!("{}", "=".repeat(60));
    println!("构建 Rust 源码 (debug模式)...");
    println!("{}", "=".repeat(60));
    
    let status = Command::new("cargo")
        .arg("build")
        .status()
        .expect("Failed to run cargo build");
    
    if status.success() {
        println!("✅ Rust 源码构建成功");
    } else {
        std::process::exit(1);
    }
}

fn build_rust_release() {
    println!("{}", "=".repeat(60));
    println!("构建 Rust 源码 (release模式)...");
    println!("{}", "=".repeat(60));
    
    let status = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .status()
        .expect("Failed to run cargo build --release");
    
    if status.success() {
        println!("✅ Rust 源码构建成功");
    } else {
        std::process::exit(1);
    }
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
    
    build_rust();
    println!();
    build_ldb();
    
    println!();
    println!("{}", "=".repeat(60));
    println!("✅ 所有构建完成");
    println!("{}", "=".repeat(60));
}
