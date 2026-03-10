use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_dir = if PathBuf::from("../../api").exists() {
        "../../api"
    } else if PathBuf::from("../api").exists() {
        "../api"
    } else {
        panic!("Could not find api directory. Expected at ../../api or ../api");
    };

    println!("cargo:rerun-if-changed={}/process.proto", api_dir);
    println!("cargo:rerun-if-changed={}/vm.proto", api_dir);
    println!("cargo:rerun-if-changed={}/provision.proto", api_dir);
    println!("cargo:rerun-if-changed={}/auth.proto", api_dir);
    println!("cargo:rerun-if-changed={}/security.proto", api_dir);
    println!("cargo:rerun-if-changed={}/log.proto", api_dir);

    tonic_prost_build::compile_protos(format!("{}/process.proto", api_dir))?;
    tonic_prost_build::compile_protos(format!("{}/vm.proto", api_dir))?;
    tonic_prost_build::compile_protos(format!("{}/provision.proto", api_dir))?;
    tonic_prost_build::compile_protos(format!("{}/auth.proto", api_dir))?;
    tonic_prost_build::compile_protos(format!("{}/security.proto", api_dir))?;
    tonic_prost_build::compile_protos(format!("{}/log.proto", api_dir))?;

    Ok(())
}
