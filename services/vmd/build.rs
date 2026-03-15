fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_prost_build::configure()
        .build_server(true)
        .build_client(false)
        .compile_protos(&["../../api/vm.proto"], &["../../api"])?;

    tonic_prost_build::configure()
        .build_server(false)
        .build_client(true)
        .compile_protos(
            &["../../api/internal/network.proto"],
            &["../../api/internal"],
        )?;

    Ok(())
}
