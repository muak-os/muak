use std::time::{Duration, Instant};

use super::service::{ServiceState, ServiceStatus};

const RESTART_DELAY: Duration = Duration::from_secs(1);
const MAX_RESTART_ATTEMPTS: u32 = 5;
const RESTART_WINDOW: Duration = Duration::from_secs(60);

/// A pending restart entry with a scheduled time.
struct PendingRestart {
    name: &'static str,
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
    /// current state, restart count, exit code, and the restart window.
    pub fn should_restart(state: &ServiceState, exit_code: Option<i32>) -> bool {
        if state.status == ServiceStatus::Stopping {
            return false;
        }

        if exit_code == Some(0) {
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
            state.service.name,
            state.restart_count,
            MAX_RESTART_ATTEMPTS,
            RESTART_DELAY
        );

        self.pending.push(PendingRestart {
            name: state.service.name,
            due_at: Instant::now() + RESTART_DELAY,
        });
    }

    /// Marks a service as permanently failed.
    pub fn mark_failed(state: &mut ServiceState) {
        state.status = ServiceStatus::Failed;
        kmsg::error!(
            "Service {} failed permanently after {} restarts",
            state.service.name,
            state.restart_count
        );
    }

    /// Returns the names of services whose restart delay has elapsed.
    pub fn take_due(&mut self, deps_ready: impl Fn(&str) -> bool) -> Vec<&'static str> {
        let now = Instant::now();
        let prev = std::mem::take(&mut self.pending);
        let mut ready = Vec::new();

        for restart in prev {
            if now < restart.due_at {
                self.pending.push(restart);
                continue;
            }

            if !deps_ready(restart.name) {
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::supervisor::service::{Service, ServiceState, ServiceStatus};

    fn make_state(status: ServiceStatus) -> ServiceState {
        let svc = Service {
            name: "test-svc",
            command: vec![],
            depends_on: vec![],
        };
        let mut state = ServiceState::new(svc);
        state.status = status;
        state
    }

    #[test]
    fn stopping_never_restarts() {
        let state = make_state(ServiceStatus::Stopping);
        assert!(!RestartQueue::should_restart(&state, None));
    }

    #[test]
    fn clean_exit_never_restarts() {
        let state = make_state(ServiceStatus::Failed);
        assert!(!RestartQueue::should_restart(&state, Some(0)));
    }

    #[test]
    fn fresh_service_restarts_on_failure() {
        let state = make_state(ServiceStatus::Failed);
        assert!(RestartQueue::should_restart(&state, Some(1)));
    }

    #[test]
    fn fresh_service_restarts_with_no_exit_code() {
        let state = make_state(ServiceStatus::Failed);
        assert!(RestartQueue::should_restart(&state, None));
    }

    #[test]
    fn at_max_attempts_does_not_restart() {
        let mut state = make_state(ServiceStatus::Failed);
        state.restart_count = MAX_RESTART_ATTEMPTS;
        state.last_restart = Some(Instant::now());
        assert!(!RestartQueue::should_restart(&state, Some(1)));
    }

    #[test]
    fn below_max_attempts_restarts() {
        let mut state = make_state(ServiceStatus::Failed);
        state.restart_count = MAX_RESTART_ATTEMPTS - 1;
        state.last_restart = Some(Instant::now());
        assert!(RestartQueue::should_restart(&state, Some(1)));
    }

    #[test]
    fn restart_count_resets_after_window() {
        let mut state = make_state(ServiceStatus::Failed);
        // Exhaust restart attempts but set last_restart far in the past (> RESTART_WINDOW).
        state.restart_count = MAX_RESTART_ATTEMPTS;
        state.last_restart = Some(Instant::now() - RESTART_WINDOW - Duration::from_secs(1));
        // Window has elapsed → count should be treated as reset → true
        assert!(RestartQueue::should_restart(&state, Some(1)));
    }

    // schedule

    #[test]
    fn schedule_increments_count_and_sets_pending() {
        let mut queue = RestartQueue::new();
        let mut state = make_state(ServiceStatus::Failed);
        assert_eq!(state.restart_count, 0);
        queue.schedule(&mut state);
        assert_eq!(state.restart_count, 1);
        assert_eq!(state.status, ServiceStatus::Pending);
        assert!(state.last_restart.is_some());
    }

    #[test]
    fn schedule_adds_to_pending_queue() {
        let mut queue = RestartQueue::new();
        let mut state = make_state(ServiceStatus::Failed);
        queue.schedule(&mut state);
        assert_eq!(queue.pending.len(), 1);
        assert_eq!(queue.pending[0].name, "test-svc");
    }

    // mark_failed

    #[test]
    fn mark_failed_sets_status() {
        let mut state = make_state(ServiceStatus::Failed);
        state.restart_count = 5;
        RestartQueue::mark_failed(&mut state);
        assert_eq!(state.status, ServiceStatus::Failed);
    }

    // take_due

    #[test]
    fn take_due_empty_queue_returns_empty() {
        let mut queue = RestartQueue::new();
        let due = queue.take_due(|_| true);
        assert!(due.is_empty());
    }

    #[test]
    fn take_due_not_yet_due_stays_pending() {
        // schedule() sets due_at = now + RESTART_DELAY (1s), so immediately after scheduling
        // the entry is not yet due.
        let mut queue = RestartQueue::new();
        let mut state = make_state(ServiceStatus::Failed);
        queue.schedule(&mut state);
        // Immediately call take_due — the 1s delay has not elapsed yet.
        let due = queue.take_due(|_| true);
        assert!(due.is_empty());
        // The entry is re-queued.
        assert_eq!(queue.pending.len(), 1);
    }

    #[test]
    fn take_due_elapsed_with_deps_ready_returns_name() {
        // Use a zero-duration delay by manipulating the pending list directly.
        // #[cfg(test)] is a child module so private fields are accessible.
        let mut queue = RestartQueue::new();
        queue.pending.push(PendingRestart {
            name: "test-svc",
            due_at: Instant::now() - Duration::from_millis(1),
        });
        let due = queue.take_due(|_| true);
        assert_eq!(due, vec!["test-svc"]);
        assert!(queue.pending.is_empty());
    }

    #[test]
    fn take_due_elapsed_but_deps_not_ready_re_queues() {
        let mut queue = RestartQueue::new();
        queue.pending.push(PendingRestart {
            name: "test-svc",
            due_at: Instant::now() - Duration::from_millis(1),
        });
        let due = queue.take_due(|_| false);
        assert!(due.is_empty());
        assert_eq!(queue.pending.len(), 1);
    }
}
