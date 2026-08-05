fn main() {
    println!("cargo:rerun-if-changed=proto/embedding.proto");
    println!("cargo:rerun-if-changed=proto/embedding_storage.proto");

    tonic_build::configure()
        .build_server(false)
        .build_client(true)
        .compile(&["proto/embedding.proto", "proto/embedding_storage.proto"], &["proto/"])
        .unwrap();
}