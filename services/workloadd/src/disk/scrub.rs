//! Btrfs periodic scrub.

extern crate alloc;

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;

use btrfs::ioctl::BtrfsScrubProgress;
use btrfs::scrub::{get_fs_info, scrub};
use tokio::task::spawn_blocking;
use tokio::time::interval;

use super::DATA_DIR;

/// Result of scrubbing a single device.
struct DeviceResult {
    progress: BtrfsScrubProgress,
}

/// Outcome of a full scrub across all devices.
enum ScrubOutcome {
    Finished(Vec<DeviceResult>),
    Error(String),
}

/// Run a read-only scrub across every device on `DATA_DIR`.
fn run() -> ScrubOutcome {
    let device_ids = match get_fs_info(DATA_DIR) {
        Ok(ids) => ids,
        Err(e) => return ScrubOutcome::Error(format!("Failed to enumerate devices: {e}")),
    };

    if device_ids.is_empty() {
        return ScrubOutcome::Error("No devices found".to_owned());
    }

    let mut results = Vec::with_capacity(device_ids.len());

    for devid in &device_ids {
        match scrub(DATA_DIR, *devid, true) {
            Ok(progress) => results.push(DeviceResult { progress }),
            Err(e) => {
                return ScrubOutcome::Error(format!("Scrub failed on device {devid}: {e}"));
            }
        }
    }

    ScrubOutcome::Finished(results)
}

fn report_outcome(outcome: ScrubOutcome) {
    match outcome {
        ScrubOutcome::Finished(devices) => {
            let total_errors: u64 = devices
                .iter()
                .map(|device| device.progress.total_errors())
                .sum();
            let total_bytes: u64 = devices
                .iter()
                .map(|device| {
                    device
                        .progress
                        .data_bytes_scrubbed
                        .saturating_add(device.progress.tree_bytes_scrubbed)
                })
                .sum();
            let gib = format_gib(total_bytes);

            if total_errors == 0 {
                println!(
                    "Scrub completed: {} device(s), {gib} GiB verified, no errors",
                    devices.len(),
                );
            } else {
                println!(
                    "Scrub completed with errors: {} device(s), {total_errors} total errors, \
                     {gib} GiB verified",
                    devices.len(),
                );
            }
        }
        ScrubOutcome::Error(msg) => {
            eprintln!("Scrub failed: {msg}");
        }
    }
}

/// Formats bytes as GiB with two decimal places.
fn format_gib(bytes: u64) -> String {
    const GIB: u64 = 1024 * 1024 * 1024;
    let mut whole = bytes.div_euclid(GIB);
    let mut fraction = bytes
        .rem_euclid(GIB)
        .wrapping_mul(100)
        .wrapping_mul(2)
        .wrapping_add(GIB)
        .div_euclid(GIB.wrapping_mul(2));
    if fraction >= 100 {
        fraction = fraction.rem_euclid(100);
        whole = whole.saturating_add(1);
    }

    format!("{whole}.{fraction:02}")
}

/// Periodic scrub timer. Runs a read-only btrfs scrub on the data partition.
pub async fn timer(shutdown: Arc<AtomicBool>) {
    const SCRUB_INTERVAL: Duration = Duration::from_hours(168);

    let mut interval = interval(SCRUB_INTERVAL);
    interval.tick().await;

    println!("Scheduled periodic btrfs scrub");

    loop {
        interval.tick().await;

        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        println!("Starting periodic btrfs scrub");

        let result = spawn_blocking(run).await;

        match result {
            Ok(outcome) => report_outcome(outcome),
            Err(e) => eprintln!("Scrub task panicked: {e}"),
        }
    }
}
