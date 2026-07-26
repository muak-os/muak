use std::fs::OpenOptions;
use std::io::{Seek, Write};

use anyhow::Result;

use super::constants::MB;

/// Wipes the first and last portions of a disk to remove partition tables.
pub fn wipe(disk: &str) -> Result<()> {
    let mut f = OpenOptions::new().read(true).write(true).open(disk)?;

    let disk_size = f.seek(std::io::SeekFrom::End(0))?;

    f.seek(std::io::SeekFrom::Start(0))?;
    f.write_all(&vec![0u8; (10 * MB) as usize])?;

    if disk_size > MB {
        f.seek(std::io::SeekFrom::Start(disk_size - MB))?;
        f.write_all(&vec![0u8; MB as usize])?;
    }

    f.sync_all()?;

    Ok(())
}
