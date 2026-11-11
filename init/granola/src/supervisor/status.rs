use std::collections::HashMap;

use tokio::sync::watch;

use super::service::{ServiceState, ServiceStatus};

/// Live report of a single supervised service.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct View {
    pub name: String,
    pub status: ServiceStatus,
    pub restarts: u32,
}

/// Ordered report of every supervised service.
pub(crate) type Snapshot = Vec<View>;

/// Creates the watch channel that carries the live snapshot.
pub(crate) fn channel() -> (watch::Sender<Snapshot>, watch::Receiver<Snapshot>) {
    watch::channel(Snapshot::new())
}

/// Publishes a name-sorted snapshot, but only when it differs from the stored one.
pub(crate) fn publish(services: &HashMap<String, ServiceState>, tx: &watch::Sender<Snapshot>) {
    let mut snapshot: Snapshot = services
        .iter()
        .map(|(name, state)| View {
            name: name.clone(),
            status: state.status.clone(),
            restarts: state.restart_count,
        })
        .collect();
    snapshot.sort_by(|left, right| left.name.cmp(&right.name));

    let unchanged = *tx.borrow() == snapshot;
    if unchanged {
        return;
    }

    tx.send_replace(snapshot);
}

#[cfg(test)]
mod tests {
    use super::super::service::Service;
    use super::*;

    fn make_service(name: &str) -> Service {
        Service {
            name: name.to_owned(),
            command: String::new(),
            depends_on: Vec::new(),
        }
    }

    fn make_services() -> HashMap<String, ServiceState> {
        let mut beta = ServiceState::new(make_service("beta"));
        beta.status = ServiceStatus::Ready;
        beta.restart_count = 2;

        let mut alpha = ServiceState::new(make_service("alpha"));
        alpha.status = ServiceStatus::Failed;

        let mut services = HashMap::new();
        services.insert("beta".to_owned(), beta);
        services.insert("alpha".to_owned(), alpha);
        services
    }

    #[test]
    fn publish_sorts_by_name_and_reports_state() {
        // ARRANGE
        let services = make_services();
        let (tx, rx) = channel();

        // ACT
        publish(&services, &tx);

        // ASSERT
        assert_eq!(
            *rx.borrow(),
            vec![
                View {
                    name: "alpha".to_owned(),
                    status: ServiceStatus::Failed,
                    restarts: 0,
                },
                View {
                    name: "beta".to_owned(),
                    status: ServiceStatus::Ready,
                    restarts: 2,
                },
            ]
        );
    }

    #[test]
    fn publish_skips_unchanged_snapshot() {
        // ARRANGE
        let services = make_services();
        let (tx, mut rx) = channel();
        publish(&services, &tx);
        rx.borrow_and_update();

        // ACT
        publish(&services, &tx);

        // ASSERT
        assert!(!rx.has_changed().expect("sender alive"));
    }

    #[test]
    fn display_maps_every_variant() {
        // ARRANGE
        let cases = [
            (ServiceStatus::Pending, "pending"),
            (ServiceStatus::Starting, "starting"),
            (ServiceStatus::Ready, "ready"),
            (ServiceStatus::Degraded, "degraded"),
            (ServiceStatus::Stopping, "stopping"),
            (ServiceStatus::Failed, "failed"),
        ];

        // ACT
        let rendered: Vec<String> = cases.iter().map(|pair| pair.0.to_string()).collect();

        // ASSERT
        let expected: Vec<String> = cases.iter().map(|pair| pair.1.to_owned()).collect();
        assert_eq!(rendered, expected);
    }
}
