//! System reboot scheduling.

use rustix::fs::sync;
use rustix::system::{RebootCommand, reboot};

/// Schedules a reboot after the given number of seconds.
pub fn schedule(delay: u64) {
    tokio::spawn(async move {
        kmsg::info!("System will reboot in {} seconds...", delay);
        tokio::time::sleep(core::time::Duration::from_secs(delay)).await;

        kmsg::info!("Rebooting now...");
        drop(
            tokio::task::spawn_blocking(|| {
                sync();
                let _result = reboot(RebootCommand::Restart);
            })
            .await,
        );
    });
}
