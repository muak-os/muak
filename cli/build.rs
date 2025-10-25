fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::compile_protos("../proto/process.proto")?;
    tonic_build::compile_protos("../proto/vm.proto")?;
    Ok(())
}
