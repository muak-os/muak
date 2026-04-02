use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_dir = if PathBuf::from("../../api").exists() {
        "../../api"
    } else if PathBuf::from("../api").exists() {
        "../api"
    } else {
        panic!("Could not find api directory. Expected at ../../api or ../api");
    };

    println!("cargo:rerun-if-changed={}/provision.proto", api_dir);
    println!("cargo:rerun-if-changed={}/auth.proto", api_dir);
    println!("cargo:rerun-if-changed={}/security.proto", api_dir);
    println!("cargo:rerun-if-changed={}/version.proto", api_dir);

    tonic_prost_build::configure()
        .build_server(true)
        .build_client(false)
        .compile_fds(protox::compile(
            [
                format!("{}/provision.proto", api_dir),
                format!("{}/auth.proto", api_dir),
                format!("{}/security.proto", api_dir),
                format!("{}/version.proto", api_dir),
            ],
            [api_dir],
        )?)?;

    Ok(())
}
