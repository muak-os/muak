pub mod process;
pub mod provision;

pub mod proto {
    pub mod process {
        tonic::include_proto!("muak.process.v1");
    }
    pub mod provision {
        tonic::include_proto!("muak.provision.v1");
    }
}
