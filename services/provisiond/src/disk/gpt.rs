//! GPT verification helpers for the plan executor.

use std::fs::OpenOptions;
use std::time::{SystemTime, UNIX_EPOCH};

use partdev::table;

/// Logs a summary of the GPT currently on `disk`.
pub(super) fn verify(disk: &str) {
    match OpenOptions::new().read(true).open(disk) {
        Ok(file) => match table::read(&file) {
            Ok(gpt) => {
                let count = gpt.used_partitions().len();
                kmsg::info!("Verified: GPT on {} has {} used partitions", disk, count);
            }
            Err(e) => kmsg::warn!("Could not verify GPT on {}: {}", disk, e),
        },
        Err(e) => kmsg::warn!("Could not open {} for GPT verification: {}", disk, e),
    }
}

/// Returns a timestamp for deterministic-enough v7 partition GUIDs.
pub(super) fn uuid_now() -> uuid::Timestamp {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();

    uuid::Timestamp::from_unix(uuid::NoContext, dur.as_secs(), dur.subsec_nanos())
}
