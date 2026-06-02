fn main() {
    let mut config = prost_build::Config::new();
    config.protoc_arg("--experimental_allow_proto3_optional");
    
    println!("cargo:rerun-if-changed=proto/metadata.proto");
    println!("cargo:rerun-if-changed=proto/row.proto");
    println!("cargo:rerun-if-changed=proto/field.proto");
    println!("cargo:rerun-if-changed=proto/query.proto");
    
    config.compile_protos(
        &[
            "proto/metadata.proto",
            "proto/row.proto",
            "proto/field.proto",
            "proto/query.proto",
        ],
        &["proto/"],
    ).expect("Failed to compile protobuf");
}
