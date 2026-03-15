use anyhow::Result;
use tokio::sync::mpsc;

use super::commands::NetworkCommand;
use super::state::NetworkActor;
use crate::model::NetworkStateKind;

impl NetworkActor {
    pub(super) async fn initialize(&mut self, cmd_tx: &mpsc::Sender<NetworkCommand>) -> Result<()> {
        kmsg::info!("Initializing network");

        self.discover_interfaces().await?;
        self.apply_interface_configs(cmd_tx).await?;

        self.state.state = NetworkStateKind::Ready;
        self.publish_state();

        self.start_connectivity_monitoring(cmd_tx.clone());

        kmsg::info!("Network initialization complete");

        Ok(())
    }
}
