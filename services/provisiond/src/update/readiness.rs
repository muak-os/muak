//! Post-boot gate: every supervised service must be ready or exit cleanly.

use core::future::Future;
use core::time::Duration;
use std::time::Instant;

use anyhow::{Context as _, Result, bail};
use tokio::time::{sleep, timeout};
use tonic::Request;
use tonic::transport::{Channel, Endpoint};

use crate::ipc::proto::service::service_state_service_client::ServiceStateServiceClient;
use crate::ipc::proto::service::{ListServiceStatesRequest, ServiceState};

const WINDOW: Duration = Duration::from_mins(4);
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const GRANOLA_SOCKET: &str = "unix:/run/services/granola.sock";
const READY: &str = "ready";
const STOPPING: &str = "stopping";

/// Source of the supervised service states reported by granola.
pub(crate) trait States {
    /// Fetches the current state of every supervised service.
    fn list(&mut self) -> impl Future<Output = Result<Vec<ServiceState>>> + Send;
}

impl States for ServiceStateServiceClient<Channel> {
    async fn list(&mut self) -> Result<Vec<ServiceState>> {
        let request = Request::new(ListServiceStatesRequest {});
        let response = self
            .list_service_states(request)
            .await
            .context("List service states")?;

        Ok(response.into_inner().states)
    }
}

struct Granola {
    client: Option<ServiceStateServiceClient<Channel>>,
}

impl Granola {
    async fn connected(&mut self) -> Result<&mut ServiceStateServiceClient<Channel>> {
        if self.client.is_none() {
            let channel = Endpoint::from_static(GRANOLA_SOCKET)
                .connect()
                .await
                .context("Connect to granola")?;

            self.client = Some(ServiceStateServiceClient::new(channel));
        }

        self.client.as_mut().context("Granola client is connected")
    }
}

impl States for Granola {
    async fn list(&mut self) -> Result<Vec<ServiceState>> {
        self.connected().await?.list().await
    }
}

/// Waits until granola reports every supervised service as ready or cleanly exited.
///
/// # Errors
///
/// Returns an error when granola is unreachable or the readiness window expires.
pub(crate) async fn await_all_ready() -> Result<()> {
    let mut states = Granola { client: None };

    wait_until_ready(&mut states, WINDOW).await
}

/// Polls `states` every [`POLL_INTERVAL`] until the gate passes or `window` expires.
///
/// Each poll is bounded by the remaining window, so a stalled connect or request
/// cannot outlive `window`.
///
/// # Errors
///
/// Returns an error once `window` elapsed, carrying the last error and the last
/// observed non-ready services.
pub(crate) async fn wait_until_ready<S: States>(states: &mut S, window: Duration) -> Result<()> {
    let deadline = Instant::now()
        .checked_add(window)
        .context("Readiness deadline overflow")?;
    let mut last_error: Option<String> = None;
    let mut last_offenders: Option<String> = None;

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return window_expired(window, last_error, last_offenders);
        }

        let budget = POLL_INTERVAL.min(remaining);
        match timeout(budget, states.list()).await {
            Ok(Ok(observed)) => match assess(&observed) {
                Ok(()) => return Ok(()),
                Err(e) => last_offenders = Some(format!("{e:#}")),
            },
            Ok(Err(e)) => last_error = Some(format!("{e:#}")),
            Err(_) => last_error = Some(format!("Poll timed out after {budget:?}")),
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        sleep(POLL_INTERVAL.min(remaining)).await;
    }
}

/// Returns an error when any service is not ready, naming the offenders.
///
/// A service satisfies the gate when it reports `ready`, or when it exited
/// cleanly: granola records a zero exit code as `stopping` with no restarts, so
/// a oneshot-style service that completed has not failed to start. A service
/// that stopped after any restart still fails the gate.
pub(crate) fn assess(states: &[ServiceState]) -> Result<()> {
    if states.is_empty() {
        bail!("no services reported");
    }

    let offenders = offenders(states);
    if offenders.is_empty() {
        return Ok(());
    }

    bail!("{}", offenders.join(", "))
}

fn offenders(states: &[ServiceState]) -> Vec<String> {
    states
        .iter()
        .filter(|state| !satisfied(state))
        .map(describe)
        .collect()
}

fn satisfied(state: &ServiceState) -> bool {
    state.status == READY || (state.status == STOPPING && state.restarts == 0)
}

fn describe(state: &ServiceState) -> String {
    let name = state.name.as_str();
    let status = state.status.as_str();

    format!("{name}={status}")
}

fn window_expired(
    window: Duration,
    last_error: Option<String>,
    last_offenders: Option<String>,
) -> Result<()> {
    let mut reason = last_offenders.unwrap_or_else(|| "no services observed".to_owned());
    if let Some(error) = last_error {
        reason = format!("{reason}; last error: {error}");
    }

    bail!("Services not ready within {window:?}: {reason}")
}

#[cfg(test)]
mod tests {
    use alloc::collections::VecDeque;
    use core::future::{pending, ready};

    use anyhow::anyhow;

    use super::*;

    // A window spanning one poll interval plus a wide margin, so a second poll
    // always happens before the deadline even under scheduling overshoot.
    const TWO_POLLS: Duration = POLL_INTERVAL.saturating_add(Duration::from_secs(1));

    struct Fake {
        polls: VecDeque<Result<Vec<ServiceState>>>,
    }

    impl Fake {
        fn new(polls: Vec<Result<Vec<ServiceState>>>) -> Self {
            Self {
                polls: polls.into(),
            }
        }
    }

    impl States for Fake {
        fn list(&mut self) -> impl Future<Output = Result<Vec<ServiceState>>> {
            let poll = match self.polls.pop_front() {
                Some(poll) => poll,
                None => Err(anyhow!("Fake polls exhausted")),
            };

            ready(poll)
        }
    }

    struct Stalled;

    impl States for Stalled {
        fn list(&mut self) -> impl Future<Output = Result<Vec<ServiceState>>> + Send {
            pending()
        }
    }

    struct Unreachable;

    impl States for Unreachable {
        fn list(&mut self) -> impl Future<Output = Result<Vec<ServiceState>>> + Send {
            ready(Err(anyhow!("granola unreachable")))
        }
    }

    fn state(name: &str, status: &str) -> ServiceState {
        ServiceState {
            name: name.to_owned(),
            status: status.to_owned(),
            restarts: 0,
        }
    }

    fn stopped(name: &str, restarts: u32) -> ServiceState {
        ServiceState {
            restarts,
            ..state(name, "stopping")
        }
    }

    #[test]
    fn assess_accepts_all_ready_states() {
        // ARRANGE
        let states = [state("apid", "ready"), state("timed", "ready")];

        // ACT
        let outcome = assess(&states);

        // ASSERT
        outcome.expect("all ready states must pass");
    }

    #[test]
    fn assess_accepts_clean_exits() {
        // ARRANGE
        let states = [state("apid", "ready"), stopped("consoled", 0)];

        // ACT
        let outcome = assess(&states);

        // ASSERT
        outcome.expect("a clean exit must satisfy the gate");
    }

    #[test]
    fn assess_rejects_stopping_after_a_restart() {
        // ARRANGE
        let states = [state("apid", "ready"), stopped("consoled", 1)];

        // ACT
        let error = assess(&states).expect_err("a restarted service must be rejected");

        // ASSERT
        assert_eq!(error.to_string(), "consoled=stopping");
    }

    #[test]
    fn assess_rejects_unhealthy_statuses() {
        // ARRANGE
        let statuses = ["degraded", "starting", "pending", "failed"];

        for status in statuses {
            // ACT
            let outcome = assess(&[state("apid", status)]);

            // ASSERT
            let error = outcome.expect_err("non-ready status must be rejected");
            assert!(
                error.to_string().contains(&format!("apid={status}")),
                "error '{error}' must name 'apid={status}'"
            );
        }
    }

    #[test]
    fn assess_rejects_mixed_states_naming_offenders() {
        // ARRANGE
        let states = [
            state("apid", "ready"),
            state("timed", "degraded"),
            state("networkd", "starting"),
        ];

        // ACT
        let error = assess(&states).expect_err("mixed states must be rejected");

        // ASSERT
        assert_eq!(error.to_string(), "timed=degraded, networkd=starting");
    }

    #[test]
    fn assess_rejects_empty_states() {
        // ARRANGE
        let states: [ServiceState; 0] = [];

        // ACT
        let error = assess(&states).expect_err("empty states must be rejected");

        // ASSERT
        assert!(error.to_string().contains("no services reported"));
    }

    #[tokio::test]
    async fn wait_until_ready_succeeds_on_the_first_poll() {
        // ARRANGE
        let mut fake = Fake::new(vec![Ok(vec![state("apid", "ready")])]);

        // ACT
        let outcome = wait_until_ready(&mut fake, Duration::from_millis(100)).await;

        // ASSERT
        outcome.expect("an all ready first poll must pass without waiting");
        assert!(fake.polls.is_empty(), "the single poll must be consumed");
    }

    #[tokio::test]
    async fn wait_until_ready_succeeds_on_a_later_poll() {
        // ARRANGE
        let mut fake = Fake::new(vec![
            Ok(vec![state("apid", "starting")]),
            Ok(vec![state("apid", "ready")]),
        ]);

        // ACT
        let outcome = wait_until_ready(&mut fake, TWO_POLLS).await;

        // ASSERT
        outcome.expect("a later poll is all ready");
        assert!(fake.polls.is_empty(), "both polls must be consumed");
    }

    #[tokio::test]
    async fn wait_until_ready_expires_naming_offenders_and_last_error() {
        // ARRANGE
        let mut fake = Fake::new(vec![
            Err(anyhow!("granola unreachable")),
            Ok(vec![state("apid", "degraded")]),
        ]);

        // ACT
        let error = wait_until_ready(&mut fake, TWO_POLLS)
            .await
            .expect_err("services never become ready");

        // ASSERT
        let message = error.to_string();
        assert!(
            message.contains("within"),
            "error '{message}' must mention the window"
        );
        assert!(
            message.contains("apid=degraded"),
            "error '{message}' must name the offender"
        );
        assert!(
            message.contains("last error") && message.contains("granola unreachable"),
            "error '{message}' must carry the last error"
        );
    }

    #[tokio::test]
    async fn wait_until_ready_retries_connection_errors() {
        // ARRANGE
        let mut fake = Fake::new(vec![
            Err(anyhow!("granola unreachable")),
            Ok(vec![state("apid", "ready")]),
        ]);

        // ACT
        let outcome = wait_until_ready(&mut fake, TWO_POLLS).await;

        // ASSERT
        outcome.expect("connection errors must not be fatal");
        assert!(fake.polls.is_empty(), "every poll must be consumed");
    }

    #[tokio::test]
    async fn wait_until_ready_expires_when_a_poll_stalls() {
        // ARRANGE
        let mut stalled = Stalled;

        // ACT
        let outcome = timeout(
            Duration::from_secs(2),
            wait_until_ready(&mut stalled, Duration::from_millis(150)),
        )
        .await;

        // ASSERT
        let error = outcome
            .expect("the poll must be bounded by the window, not by the stalled call")
            .expect_err("a stalled poll must expire the window");
        let message = error.to_string();
        assert!(
            message.contains("no services observed"),
            "error '{message}' must report no observed services"
        );
        assert!(
            message.contains("last error") && message.contains("Poll timed out"),
            "error '{message}' must carry the poll timeout"
        );
    }

    #[tokio::test]
    async fn wait_until_ready_expires_without_observing_services() {
        // ARRANGE
        let mut unreachable = Unreachable;

        // ACT
        let error = wait_until_ready(&mut unreachable, Duration::from_millis(150))
            .await
            .expect_err("services are never observed");

        // ASSERT
        let message = error.to_string();
        assert!(
            message.contains("no services observed"),
            "error '{message}' must report no observed services"
        );
        assert!(
            message.contains("last error") && message.contains("granola unreachable"),
            "error '{message}' must carry the last error"
        );
    }
}
