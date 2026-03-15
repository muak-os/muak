//! gRPC service implementations for provisiond.

pub mod auth;
pub mod provision;
pub mod security;
pub mod version;

#[allow(clippy::excessive_nesting)]
pub mod proto {
    pub mod auth {
        tonic::include_proto!("muak.auth.v1");
    }
    pub mod provision {
        tonic::include_proto!("muak.provision.v1");
    }
    pub mod security {
        tonic::include_proto!("muak.security.v1");
    }
    pub mod version {
        tonic::include_proto!("muak.version.v1");
    }
}
