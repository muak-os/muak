//! System power state transitions (reboot and power-off).

use core::time::Duration;

use rustix::system::RebootCommand;

/// Schedules a reboot after the given number of seconds.
pub fn reboot(delay: u64) {
    schedule("reboot", RebootCommand::Restart, delay);
}

/// Schedules a power-off after the given number of seconds.
pub fn poweroff(delay: u64) {
    schedule("power off", RebootCommand::PowerOff, delay);
}

fn schedule(action: &'static str, command: RebootCommand, delay: u64) {
    tokio::spawn(async move {
        kmsg::info!("System will {action} in {delay} seconds...");
        tokio::time::sleep(Duration::from_secs(delay)).await;

        kmsg::info!("{action} now...");
        drop(
            tokio::task::spawn_blocking(move || {
                rustix::fs::sync();
                let _result = rustix::system::reboot(command);
            })
            .await,
        );
    });
}
