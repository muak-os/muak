use std::time::{Duration, Instant};

use super::service::{ServiceState, ServiceStatus};

const RESTART_DELAY: Duration = Duration::from_secs(1);
const MAX_RESTART_ATTEMPTS: u32 = 5;
const RESTART_WINDOW: Duration = Duration::from_secs(60);

/// A pending restart entry with a scheduled time.
struct PendingRestart {
    name: String,
    due_at: Instant,
}

/// Manages restarts scheduling and policy decisions.
pub struct RestartQueue {
    pending: Vec<PendingRestart>,
}

impl RestartQueue {
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    /// Determines whether a service should be restarted based on its
    /// current state, restart count, and the restart window.
    pub fn should_restart(state: &ServiceState) -> bool {
        if state.status == ServiceStatus::Stopping {
            return false;
        }

        if let Some(last) = state.last_restart
            && last.elapsed() > RESTART_WINDOW
        {
            return true;
        }

        state.restart_count < MAX_RESTART_ATTEMPTS
    }

    /// Schedules a service for restart after the configured delay.
    pub fn schedule(&mut self, state: &mut ServiceState) {
        state.status = ServiceStatus::Pending;
        state.restart_count += 1;
        state.last_restart = Some(Instant::now());

        kmsg::info!(
            "Will restart {} (attempt {}/{}) after {:?}",
            state.def.name,
            state.restart_count,
            MAX_RESTART_ATTEMPTS,
            RESTART_DELAY
        );

        self.pending.push(PendingRestart {
            name: state.def.name.clone(),
            due_at: Instant::now() + RESTART_DELAY,
        });
    }

    /// Marks a service as permanently failed.
    pub fn mark_failed(state: &mut ServiceState) {
        state.status = ServiceStatus::Failed;
        kmsg::error!("Service {} failed permanently", state.def.name);
    }

    /// Returns the names of services whose restart delay has elapsed.
    pub fn take_due(&mut self, deps_ready: impl Fn(&str) -> bool) -> Vec<String> {
        let now = Instant::now();
        let prev = std::mem::take(&mut self.pending);
        let mut ready = Vec::new();

        for restart in prev {
            if now < restart.due_at {
                self.pending.push(restart);
                continue;
            }

            if !deps_ready(&restart.name) {
                self.pending.push(PendingRestart {
                    name: restart.name,
                    due_at: now + Duration::from_secs(1),
                });
                continue;
            }

            ready.push(restart.name);
        }

        ready
    }
}
