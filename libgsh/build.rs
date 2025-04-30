fn main() -> Result<(), Box<dyn std::error::Error>> {
    prost_build::compile_protos(&["src/shared/protocol.proto"], &["src/shared"])?;
    Ok(())
}
