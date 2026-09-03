use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_dir = if PathBuf::from("../../api").exists() {
        "../../api"
    } else if PathBuf::from("../api").exists() {
        "../api"
    } else {
        return Err("Could not find api directory. Expected at ../../api or ../api".into());
    };

    println!("cargo:rerun-if-changed={api_dir}/process.proto");
    println!("cargo:rerun-if-changed={api_dir}/log.proto");

    tonic_prost_build::configure()
        .build_server(true)
        .build_client(false)
        .compile_fds(protox::compile(
            [
                format!("{api_dir}/process.proto"),
                format!("{api_dir}/log.proto"),
            ],
            [api_dir],
        )?)?;

    Ok(())
}
