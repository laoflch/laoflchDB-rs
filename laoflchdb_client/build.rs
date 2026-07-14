fn main() {
    println!("cargo:rerun-if-changed=proto/rpc.proto");

    tonic_build::configure()
        .build_server(false)
        .build_client(true)
        .protoc_arg("--experimental_allow_proto3_optional")
        .compile(&["proto/rpc.proto"], &["proto/"])
        .unwrap();
}