use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir =
        std::env::var("CARGO_MANIFEST_DIR").expect("could not get CARGO_MANIFEST_DIR");
    let manifest_path = PathBuf::from(&manifest_dir);
    let internal_dir = manifest_path
        .parent()
        .expect("could not get parent directory");

    println!(
        "cargo:rerun-if-changed={}",
        internal_dir.join("default.toml").display()
    );

    let api_dir = if PathBuf::from("../../api").exists() {
        "../../api"
    } else if PathBuf::from("../api").exists() {
        "../api"
    } else {
        panic!("Could not find api directory. Expected at ../../api or ../api");
    };

    println!("cargo:rerun-if-changed={}/process.proto", api_dir);
    println!("cargo:rerun-if-changed={}/log.proto", api_dir);
    println!(
        "cargo:rerun-if-changed={}/internal/supervisor.proto",
        api_dir
    );

    tonic_prost_build::configure()
        .build_server(true)
        .build_client(false)
        .compile_protos(
            &[
                format!("{}/process.proto", api_dir),
                format!("{}/log.proto", api_dir),
            ],
            &[api_dir.to_string()],
        )?;

    tonic_prost_build::compile_protos(format!("{}/internal/supervisor.proto", api_dir))?;

    Ok(())
}
