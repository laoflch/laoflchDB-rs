fn main() {
    prost_build::compile_protos(
        &[
            "proto/metadata.proto",
            "proto/row.proto",
            "proto/field.proto",
        ],
        &["proto/"],
    )
    .unwrap();
}
