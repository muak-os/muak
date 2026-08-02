use core::error::Error;
use std::path::PathBuf;

fn api_dir() -> Result<&'static str, Box<dyn Error>> {
    if PathBuf::from("../../api").exists() {
        Ok("../../api")
    } else if PathBuf::from("../api").exists() {
        Ok("../api")
    } else {
        Err("Could not find api directory. Expected at ../../api or ../api".into())
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let api_dir = api_dir()?;

    println!("cargo:rerun-if-changed={api_dir}/process.proto");
    println!("cargo:rerun-if-changed={api_dir}/vm.proto");
    println!("cargo:rerun-if-changed={api_dir}/provision.proto");
    println!("cargo:rerun-if-changed={api_dir}/auth.proto");
    println!("cargo:rerun-if-changed={api_dir}/security.proto");
    println!("cargo:rerun-if-changed={api_dir}/log.proto");
    println!("cargo:rerun-if-changed={api_dir}/version.proto");

    tonic_prost_build::configure()
        .build_server(false)
        .build_client(true)
        .compile_fds(protox::compile(
            [
                format!("{api_dir}/process.proto"),
                format!("{api_dir}/vm.proto"),
                format!("{api_dir}/provision.proto"),
                format!("{api_dir}/auth.proto"),
                format!("{api_dir}/security.proto"),
                format!("{api_dir}/log.proto"),
                format!("{api_dir}/version.proto"),
            ],
            [api_dir],
        )?)?;

    Ok(())
}
