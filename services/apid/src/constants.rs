//! Static configuration constants for paths and service prefixes.

/// Certificate paths
pub const CA_CERT_PATH: &str = "/run/state/secrets/ca.crt";
pub const SERVER_CERT_PATH: &str = "/run/state/secrets/server.crt";
pub const SERVER_KEY_PATH: &str = "/run/state/secrets/server.key";

/// Backend socket paths
pub const VMD_SOCKET: &str = "/run/services/vmd.sock";
pub const GRANOLA_SOCKET: &str = "/run/services/granola.sock";
pub const PROVISIOND_SOCKET: &str = "/run/services/provisiond.sock";

/// gRPC service prefixes (used for routing to backends)
pub const VM_SERVICE_PREFIX: &str = "/muak.vm.v1.VmService/";
pub const PROCESS_SERVICE_PREFIX: &str = "/muak.process.v1.ProcessService/";
pub const PROVISION_SERVICE_PREFIX: &str = "/muak.provision.v1.ProvisionService/";
pub const AUTH_SERVICE_PREFIX: &str = "/muak.auth.v1.AuthService/";
pub const SECURITY_SERVICE_PREFIX: &str = "/muak.security.v1.SecurityService/";
pub const LOG_SERVICE_PREFIX: &str = "/muak.log.v1.LogService/";
pub const VERSION_SERVICE_PREFIX: &str = "/muak.version.v1.VersionService/";
