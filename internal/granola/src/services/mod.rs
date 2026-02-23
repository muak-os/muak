pub mod log;
pub mod process;

#[allow(clippy::excessive_nesting)]
pub mod proto {
    pub mod process {
        tonic::include_proto!("muak.process.v1");
    }
    pub mod log {
        tonic::include_proto!("muak.log.v1");
    }
}
