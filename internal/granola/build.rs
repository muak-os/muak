use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_dir = if PathBuf::from("../../api").exists() {
        "../../api"
    } else if PathBuf::from("../api").exists() {
        "../api"
    } else {
        panic!("Could not find api directory. Expected at ../../api or ../api");
    };
    tonic_build::compile_protos(format!("{}/process.proto", api_dir))?;
    tonic_build::compile_protos(format!("{}/vm.proto", api_dir))?;
    tonic_build::compile_protos(format!("{}/maintenance.proto", api_dir))?;

    Ok(())
}
