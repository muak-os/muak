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
        let mut builder = ArgvBuilder::default();
        for ch in self.command.chars() {
            builder.accept(ch);
        }
        builder.flush();
        builder.args
    }
}

/// Incremental argv builder that tracks quote state.
#[derive(Default)]
struct ArgvBuilder {
    args: Vec<String>,
    current: String,
    quote: Option<char>,
}

impl ArgvBuilder {
    /// Accumulates one command character into the builder.
    fn accept(&mut self, ch: char) {
        match self.quote {
            Some(opening) if ch == opening => self.quote = None,
            Some(_) => self.current.push(ch),
            None => self.accept_unquoted(ch),
        }
    }

    /// Accumulates one character while outside of a quoted section.
    fn accept_unquoted(&mut self, ch: char) {
        if ch == '"' || ch == '\'' {
            self.quote = Some(ch);
        } else if ch == ' ' || ch == '\t' {
            self.flush();
        } else {
            self.current.push(ch);
        }
    }

    /// Pushes the current word onto the argument list, if any.
    fn flush(&mut self) {
        if self.current.is_empty() {
            return;
        }
        self.args.push(std::mem::take(&mut self.current));
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
