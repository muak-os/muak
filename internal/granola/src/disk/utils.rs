use anyhow::{Result, bail};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::time::SystemTime;

use super::constants::MB;

pub fn format_partition_name(disk: &str, partition: u32) -> String {
    if disk.contains("nvme") || disk.contains("mmcblk") {
        format!("{}p{}", disk, partition)
    } else {
        format!("{}{}", disk, partition)
    }
}

pub fn generate_guid() -> [u8; 16] {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();

    let mut guid = [0u8; 16];
    let nanos = now.as_nanos();
    guid[0..8].copy_from_slice(&nanos.to_le_bytes()[0..8]);
    guid[8..16].copy_from_slice(&nanos.to_be_bytes()[0..8]);

    guid
}

pub fn wait_for_device(device: &str) -> Result<()> {
    for _ in 0..30 {
        if Path::new(device).exists() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    bail!("Timeout waiting for device {} to appear", device)
}

pub fn wipe_disk(disk: &str) -> Result<()> {
    let mut f = OpenOptions::new().read(true).write(true).open(disk)?;

    // Wipe first 10MB (removes any existing partition tables)
    let zeros = vec![0u8; (10 * MB) as usize];
    f.write_all(&zeros)?;
    f.sync_all()?;

    Ok(())
}
