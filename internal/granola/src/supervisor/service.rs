use std::time::Instant;

/// Blueprint for a supervised service.
#[derive(Clone, Debug)]
pub struct ServiceDef {
    pub name: String,
    pub binary: String,
    pub args: Vec<String>,
    pub depends_on: Vec<String>,
}

/// Lifecycle state of a supervised service.
#[derive(Debug, Clone, PartialEq)]
pub enum ServiceStatus {
    Pending,
    Starting,
    Ready,
    Degraded,
    Stopping,
    Failed,
}

/// Runtime state for a single supervised service.
pub struct ServiceState {
    pub def: ServiceDef,
    pub pid: Option<i32>,
    pub status: ServiceStatus,
    pub socket_path: Option<String>,
    pub restart_count: u32,
    pub last_restart: Option<Instant>,
}

impl ServiceState {
    pub fn new(def: ServiceDef) -> Self {
        Self {
            def,
            pid: None,
            status: ServiceStatus::Pending,
            socket_path: None,
            restart_count: 0,
            last_restart: None,
        }
    }
}
