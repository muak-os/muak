pub mod process;

#[allow(clippy::excessive_nesting)]
pub mod proto {
    pub mod process {
        tonic::include_proto!("muak.process.v1");
    }
}
