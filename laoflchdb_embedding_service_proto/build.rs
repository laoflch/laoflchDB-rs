fn main() {
    println!("cargo:rerun-if-changed=proto/embedding.proto");

    tonic_build::configure()
        .build_server(false)
        .build_client(true)
        .compile(&["proto/embedding.proto"], &["proto/"])
        .unwrap();
}