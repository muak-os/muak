fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_prost_build::configure()
        .build_server(true)
        .build_client(false)
        .compile_protos(
            &[
                "../../api/process.proto",
                "../../api/vm.proto",
                "../../api/maintenance.proto",
            ],
            &["../../api"],
        )?;

    Ok(())
}
