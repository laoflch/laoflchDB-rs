fn main() {
    let mut config = prost_build::Config::new();
    config.protoc_arg("--experimental_allow_proto3_optional");
    
    // 移除 laoflchdb/ 前缀，直接在同一包中构建
    config.compile_protos(
        &[
            "proto/metadata.proto",
            "proto/row.proto",
            "proto/field.proto",
            "proto/query.proto",
        ],
        &["proto/"],
    )
    .unwrap();
}
