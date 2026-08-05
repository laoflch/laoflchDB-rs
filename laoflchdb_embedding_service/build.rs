fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::compile_protos("proto/embedding.proto")?;
    tonic_build::compile_protos("proto/embedding_storage.proto")?;
    Ok(())
}