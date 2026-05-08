fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")?;
    let proto = std::path::Path::new(&manifest).join("../../proto/aether.proto");
    tonic_build::compile_protos(proto)?;
    Ok(())
}
