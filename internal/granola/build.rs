fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::compile_protos("../../api/process.proto")?;
    tonic_build::compile_protos("../../api/vm.proto")?;
    Ok(())
}
