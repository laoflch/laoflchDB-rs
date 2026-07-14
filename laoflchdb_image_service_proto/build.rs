fn main() {
    println!("cargo:rerun-if-changed=proto/image_service.proto");

    tonic_build::configure()
        .build_server(false)
        .build_client(true)
        .compile(&["proto/image_service.proto"], &["proto/"])
        .unwrap();
}