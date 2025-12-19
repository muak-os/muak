fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_prost_build::configure()
        .build_server(false)
        .build_client(false)
        .compile_protos(
            &["../../../api/internal/supervisor.proto"],
            &["../../../api/internal"],
        )?;
    Ok(())
}
