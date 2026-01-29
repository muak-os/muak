pub mod auth;
pub mod process;
pub mod provision;

#[allow(clippy::excessive_nesting)]
pub mod proto {
    pub mod auth {
        tonic::include_proto!("muak.auth.v1");
    }
    pub mod process {
        tonic::include_proto!("muak.process.v1");
    }
    pub mod provision {
        tonic::include_proto!("muak.provision.v1");
    }
}
