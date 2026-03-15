use std::time::Duration;

use tokio::sync::mpsc;

use super::commands::NetworkCommand;
use super::state::NetworkActor;
use crate::connectivity::{self, ConnectivityConfig};
use crate::model::{ConnectivityResult, ConnectivityStatus};

/// Interval between connectivity checks.
const CHECK_INTERVAL_SECS: u64 = 60;

impl NetworkActor {
    pub(super) fn start_connectivity_monitoring(&mut self, cmd_tx: mpsc::Sender<NetworkCommand>) {
        let interval = Duration::from_secs(CHECK_INTERVAL_SECS);
        let task = tokio::spawn(run_connectivity_monitor(cmd_tx, interval));
        self.connectivity_task = Some(task);
    }

    pub(super) async fn check_connectivity(&mut self) -> ConnectivityResult {
        let was_connected = self.state.connectivity.status == ConnectivityStatus::Connected;
        self.state.connectivity.status = ConnectivityStatus::Checking;
        self.publish_state();

        let cfg = ConnectivityConfig::from_network_config();
        let result = connectivity::check_connectivity(&cfg).await;

        self.state.connectivity = result.clone();
        self.publish_state();

        match result.status {
            ConnectivityStatus::Connected if !was_connected => {
                kmsg::info!("Connectivity OK ({}ms)", result.latency_ms.unwrap_or(0));
            }
            ConnectivityStatus::Disconnected => {
                kmsg::warn!("No internet connectivity detected");
            }
            _ => {}
        }

        result
    }
}

async fn run_connectivity_monitor(cmd_tx: mpsc::Sender<NetworkCommand>, interval: Duration) {
    let mut timer = tokio::time::interval_at(tokio::time::Instant::now(), interval);
    loop {
        timer.tick().await;
        if cmd_tx
            .send(NetworkCommand::PeriodicConnectivityCheck)
            .await
            .is_err()
        {
            break;
        }
    }
}
