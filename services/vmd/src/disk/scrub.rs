//! Btrfs periodic scrub.

use std::time::Duration;

use btrfs::BtrfsScrubProgress;

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
    let device_ids = match btrfs::get_fs_info(DATA_DIR) {
        Ok(ids) => ids,
        Err(e) => return ScrubOutcome::Error(format!("Failed to enumerate devices: {}", e)),
    };

    if device_ids.is_empty() {
        return ScrubOutcome::Error("No devices found".to_string());
    }

    let mut results = Vec::with_capacity(device_ids.len());

    for devid in &device_ids {
        match btrfs::scrub(DATA_DIR, *devid, true) {
            Ok(progress) => results.push(DeviceResult { progress }),
            Err(e) => {
                return ScrubOutcome::Error(format!("Scrub failed on device {}: {}", devid, e));
            }
        }
    }

    ScrubOutcome::Finished(results)
}

fn report_outcome(outcome: ScrubOutcome) {
    match outcome {
        ScrubOutcome::Finished(devices) => {
            let total_errors: u64 = devices.iter().map(|d| d.progress.total_errors()).sum();
            let total_bytes: u64 = devices
                .iter()
                .map(|d| d.progress.data_bytes_scrubbed + d.progress.tree_bytes_scrubbed)
                .sum();
            let gib = total_bytes as f64 / (1024.0 * 1024.0 * 1024.0);

            if total_errors == 0 {
                println!(
                    "Scrub completed: {} device(s), {:.2} GiB verified, no errors",
                    devices.len(),
                    gib,
                );
            } else {
                println!(
                    "Scrub completed with errors: {} device(s), {} total errors, {:.2} GiB verified",
                    devices.len(),
                    total_errors,
                    gib,
                );
            }
        }
        ScrubOutcome::Error(msg) => {
            eprintln!("Scrub failed: {}", msg);
        }
    }
}

/// Periodic scrub timer. Runs a read-only btrfs scrub on the data partition
pub async fn timer() {
    const SCRUB_INTERVAL: Duration = Duration::from_secs(7 * 24 * 60 * 60); // weekly

    let mut interval = tokio::time::interval(SCRUB_INTERVAL);
    interval.tick().await;

    println!("Scheduled periodic btrfs scrub");

    loop {
        interval.tick().await;

        println!("Starting periodic btrfs scrub");

        let result = tokio::task::spawn_blocking(run).await;

        match result {
            Ok(outcome) => report_outcome(outcome),
            Err(e) => eprintln!("Scrub task panicked: {}", e),
        }
    }
}
