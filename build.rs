fn main() {
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile(&["proto/metadata.proto", "proto/rpc.proto"], &["proto/"])
        .unwrap();
}
