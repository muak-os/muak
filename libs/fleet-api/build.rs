use core::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    for proto in ["fleet.proto", "auth.proto"] {
        println!("cargo:rerun-if-changed=../../api/{proto}");
    }
    tonic_prost_build::configure().compile_fds(protox::compile(
        ["../../api/fleet.proto", "../../api/auth.proto"],
        ["../../api"],
    )?)?;
    Ok(())
}
