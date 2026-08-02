use core::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    tonic_prost_build::configure()
        .build_server(true)
        .build_client(false)
        .compile_fds(protox::compile(["../../api/vm.proto"], ["../../api"])?)?;

    Ok(())
}
