use std::os::fd::OwnedFd;
use std::time::Instant;

use serde::Deserialize;

/// Blueprint for a supervised service.
#[derive(Clone, Debug, Deserialize)]
pub struct Service {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

impl Service {
    /// Splits `command` into an argv vector, respecting single and double quotes.
    pub fn argv(&self) -> Vec<String> {
        let mut args = Vec::new();
        let mut current = String::new();
        let mut in_quotes = false;
        let mut quote_char = '"';
        for c in self.command.chars() {
            match c {
                '"' | '\'' if !in_quotes => {
                    in_quotes = true;
                    quote_char = c;
                }
                c if in_quotes && c == quote_char => {
                    in_quotes = false;
                }
                ' ' | '\t' if !in_quotes => {
                    if !current.is_empty() {
                        args.push(std::mem::take(&mut current));
                    }
                }
                _ => current.push(c),
            }
        }
        if !current.is_empty() {
            args.push(current);
        }
        args
    }
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
    pub service: Service,
    pub pid: Option<i32>,
    pub status: ServiceStatus,
    pub listener_fd: Option<OwnedFd>,
    pub restart_count: u32,
    pub last_restart: Option<Instant>,
}

impl ServiceState {
    pub fn new(service: Service) -> Self {
        Self {
            service,
            pid: None,
            status: ServiceStatus::Pending,
            listener_fd: None,
            restart_count: 0,
            last_restart: None,
        }
    }
}
