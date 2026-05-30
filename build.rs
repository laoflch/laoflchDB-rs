fn main() {
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
}
