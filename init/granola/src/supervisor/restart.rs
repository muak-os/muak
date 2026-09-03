use std::time::{Duration, Instant};

use super::service::{ServiceState, ServiceStatus};

const RESTART_DELAY: Duration = Duration::from_secs(1);
pub const MAX_RESTART_ATTEMPTS: u32 = 5;
const RESTART_WINDOW: Duration = Duration::from_secs(60);

/// Delay applied when a restart is due but its dependencies are not ready.
const RESCHEDULE_DELAY: Duration = Duration::from_secs(1);

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
        state.restart_count = state.restart_count.saturating_add(1);
        state.last_restart = Some(Instant::now());

        kmsg::info!(
            "Will restart {} (attempt {}/{}) after {:?}",
            state.service.name,
            state.restart_count,
            MAX_RESTART_ATTEMPTS,
            RESTART_DELAY
        );

        self.pending.push(PendingRestart {
            name: state.service.name.clone(),
            due_at: Instant::now()
                .checked_add(RESTART_DELAY)
                .unwrap_or_else(Instant::now),
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
    pub fn take_due<F>(&mut self, deps_ready: F) -> Vec<String>
    where
        F: Fn(&str) -> bool,
    {
        let now = Instant::now();
        let prev = std::mem::take(&mut self.pending);
        let mut ready = Vec::new();

        for restart in prev {
            self.resolve_pending(restart, now, &deps_ready, &mut ready);
        }

        ready
    }

    /// Either re-queues a pending restart or collects it as ready to run.
    fn resolve_pending<G>(
        &mut self,
        restart: PendingRestart,
        now: Instant,
        deps_ready: &G,
        ready: &mut Vec<String>,
    ) where
        G: Fn(&str) -> bool,
    {
        if now < restart.due_at {
            self.pending.push(restart);
            return;
        }
        if !deps_ready(&restart.name) {
            self.pending.push(PendingRestart {
                name: restart.name,
                due_at: now.checked_add(RESCHEDULE_DELAY).unwrap_or(now),
            });
            return;
        }
        ready.push(restart.name);
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::supervisor::service::{Service, ServiceState, ServiceStatus};

    fn make_state(status: ServiceStatus) -> ServiceState {
        let svc = Service {
            name: "test-svc".to_owned(),
            command: String::new(),
            depends_on: vec![],
        };
        let mut state = ServiceState::new(svc);
        state.status = status;
        state
    }

    #[test]
    fn stopping_never_restarts() {
        // ARRANGE
        let state = make_state(ServiceStatus::Stopping);

        // ACT & ASSERT
        assert!(!RestartQueue::should_restart(&state, None));
    }

    #[test]
    fn clean_exit_never_restarts() {
        // ARRANGE
        let state = make_state(ServiceStatus::Failed);

        // ACT & ASSERT
        assert!(!RestartQueue::should_restart(&state, Some(0)));
    }

    #[test]
    fn fresh_service_restarts_on_failure() {
        // ARRANGE
        let state = make_state(ServiceStatus::Failed);

        // ACT & ASSERT
        assert!(RestartQueue::should_restart(&state, Some(1)));
    }

    #[test]
    fn fresh_service_restarts_with_no_exit_code() {
        // ARRANGE
        let state = make_state(ServiceStatus::Failed);

        // ACT & ASSERT
        assert!(RestartQueue::should_restart(&state, None));
    }

    #[test]
    fn at_max_attempts_does_not_restart() {
        // ARRANGE
        let mut state = make_state(ServiceStatus::Failed);
        state.restart_count = MAX_RESTART_ATTEMPTS;
        state.last_restart = Some(Instant::now());

        // ACT & ASSERT
        assert!(!RestartQueue::should_restart(&state, Some(1)));
    }

    #[test]
    fn below_max_attempts_restarts() {
        // ARRANGE
        let mut state = make_state(ServiceStatus::Failed);
        state.restart_count = MAX_RESTART_ATTEMPTS - 1;
        state.last_restart = Some(Instant::now());

        // ACT & ASSERT
        assert!(RestartQueue::should_restart(&state, Some(1)));
    }

    #[test]
    fn restart_count_resets_after_window() {
        // ARRANGE
        let mut state = make_state(ServiceStatus::Failed);
        state.restart_count = MAX_RESTART_ATTEMPTS;
        let long_ago = Instant::now()
            .checked_sub(RESTART_WINDOW)
            .and_then(|instant| instant.checked_sub(Duration::from_secs(1)));
        state.last_restart = long_ago;

        // ACT & ASSERT
        assert!(RestartQueue::should_restart(&state, Some(1)));
    }

    #[test]
    fn schedule_increments_count_and_sets_pending() {
        // ARRANGE
        let mut queue = RestartQueue::new();
        let mut state = make_state(ServiceStatus::Failed);

        // ASSERT (initial state)
        assert_eq!(state.restart_count, 0);

        // ACT
        queue.schedule(&mut state);

        // ASSERT
        assert_eq!(state.restart_count, 1);
        assert_eq!(state.status, ServiceStatus::Pending);
        assert!(state.last_restart.is_some());
    }

    #[test]
    fn schedule_adds_to_pending_queue() {
        // ARRANGE
        let mut queue = RestartQueue::new();
        let mut state = make_state(ServiceStatus::Failed);

        // ACT
        queue.schedule(&mut state);

        // ASSERT
        assert_eq!(queue.pending.len(), 1);
        assert_eq!(
            queue.pending.first().map(|entry| entry.name.as_str()),
            Some("test-svc")
        );
    }

    #[test]
    fn mark_failed_sets_status() {
        // ARRANGE
        let mut state = make_state(ServiceStatus::Failed);
        state.restart_count = 5;

        // ACT
        RestartQueue::mark_failed(&mut state);

        // ASSERT
        assert_eq!(state.status, ServiceStatus::Failed);
    }

    #[test]
    fn take_due_empty_queue_returns_empty() {
        // ARRANGE
        let mut queue = RestartQueue::new();

        // ACT
        let due = queue.take_due(|_| true);

        // ASSERT
        assert!(due.is_empty());
    }

    #[test]
    fn take_due_not_yet_due_stays_pending() {
        // ARRANGE
        let mut queue = RestartQueue::new();
        let mut state = make_state(ServiceStatus::Failed);
        queue.schedule(&mut state);

        // ACT
        let due = queue.take_due(|_| true);

        // ASSERT
        assert!(due.is_empty());
        assert_eq!(queue.pending.len(), 1);
    }

    #[test]
    fn take_due_elapsed_with_deps_ready_returns_name() {
        // ARRANGE
        let mut queue = RestartQueue::new();
        queue.pending.push(PendingRestart {
            name: "test-svc".to_owned(),
            due_at: Instant::now()
                .checked_sub(Duration::from_millis(1))
                .expect("valid instant"),
        });

        // ACT
        let due = queue.take_due(|_| true);

        // ASSERT
        assert_eq!(due, vec!["test-svc".to_owned()]);
        assert!(queue.pending.is_empty());
    }

    #[test]
    fn take_due_elapsed_but_deps_not_ready_re_queues() {
        // ARRANGE
        let mut queue = RestartQueue::new();
        queue.pending.push(PendingRestart {
            name: "test-svc".to_owned(),
            due_at: Instant::now()
                .checked_sub(Duration::from_millis(1))
                .expect("valid instant"),
        });

        // ACT
        let due = queue.take_due(|_| false);

        // ASSERT
        assert!(due.is_empty());
        assert_eq!(queue.pending.len(), 1);
    }
}
