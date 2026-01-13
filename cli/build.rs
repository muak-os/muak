use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let manifest_path = PathBuf::from(&manifest_dir);
    let workspace_root = manifest_path.parent().unwrap();

    println!(
        "cargo:rerun-if-changed={}",
        workspace_root.join("internal/default.toml").display()
    );

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

    tonic_prost_build::compile_protos(format!("{}/process.proto", api_dir))?;
    tonic_prost_build::compile_protos(format!("{}/vm.proto", api_dir))?;
    tonic_prost_build::compile_protos(format!("{}/provision.proto", api_dir))?;

    Ok(())
}
