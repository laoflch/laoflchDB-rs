fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::compile_protos("proto/face_service.proto")?;
    Ok(())
}
