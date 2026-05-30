fn main() {
    println!("cargo:rerun-if-changed=src/access/proto/rpc.proto");
    
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .include_file("pb.rs")
        .compile(
            &[
                "src/access/proto/rpc.proto",
            ],
            &[
                "src/access/proto/",
            ],
        )
        .unwrap();
    
    build_ldb();
}

fn build_ldb() {
    println!("cargo:rerun-if-changed=rocksdb/Makefile");
    println!("cargo:rerun-if-changed=rocksdb/common.mk");
    println!("cargo:rerun-if-changed=rocksdb/src.mk");
    println!("cargo:rerun-if-changed=rocksdb/tools/ldb.cc");
    println!("cargo:rerun-if-changed=rocksdb/tools/ldb_cmd.cc");
    println!("cargo:rerun-if-changed=rocksdb/tools/ldb_tool.cc");
    
    let target_dir = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".to_string());
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    let ldb_output = std::path::Path::new(&target_dir).join(&profile).join("ldb");
    
    let need_build = !ldb_output.exists() 
        || std::fs::read_dir("rocksdb")
            .ok()
            .and_then(|mut entries| {
                entries.find(|e| {
                    e.as_ref().ok()
                        .and_then(|e| e.metadata().ok())
                        .map(|m| m.modified().ok())
                        .flatten()
                        .map(|t1| {
                            ldb_output.metadata().ok()
                                .map(|m| m.modified().ok())
                                .flatten()
                                .map(|t2| t1 > t2)
                                .unwrap_or(true)
                        })
                        .unwrap_or(true)
                })
            })
            .is_some();
    
    if need_build {
        println!("cargo:warning=Building RocksDB ldb tool...");
        
        let status = std::process::Command::new("make")
            .args(&["ldb", "-j8", "DEBUG_LEVEL=0"])
            .current_dir("rocksdb")
            .env("CC", "/usr/local/gcc-13/bin/gcc")
            .env("CXX", "/usr/local/gcc-13/bin/g++")
            .env("PATH", format!("/usr/local/gcc-13/bin:{}", std::env::var("PATH").unwrap_or_default()))
            .status();
        
        match status {
            Ok(s) if s.success() => {
                println!("cargo:warning=ldb built successfully!");
                if let Err(e) = std::fs::copy("rocksdb/ldb", &ldb_output) {
                    println!("cargo:warning=Failed to copy ldb: {}", e);
                } else {
                    println!("cargo:warning=ldb copied to {}", ldb_output.display());
                }
            }
            Ok(s) => {
                println!("cargo:warning=ldb build failed with exit code: {}", s);
            }
            Err(e) => {
                println!("cargo:warning=Failed to run make: {}", e);
            }
        }
    } else {
        println!("cargo:warning=ldb already exists, skipping build");
    }
}
