use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_dir = if PathBuf::from("../../api").exists() {
        "../../api"
    } else if PathBuf::from("../api").exists() {
        "../api"
    } else {
        panic!("Could not find api directory. Expected at ../../api or ../api");
    };

    tonic_prost_build::configure()
        .build_server(true)
        .build_client(false)
        .compile_protos(
            &[
                format!("{}/process.proto", api_dir),
                format!("{}/provision.proto", api_dir),
            ],
            &[api_dir.to_string()],
        )?;

    tonic_prost_build::compile_protos(format!("{}/internal/supervisor.proto", api_dir))?;

    Ok(())
}
