use std::fs::OpenOptions;
use std::io::{Seek as _, Write as _};

use anyhow::Result;

use super::constants::MB;

/// Wipes the first and last portions of a disk to remove partition tables.
pub fn wipe(disk: &str) -> Result<()> {
    let mut file = OpenOptions::new().read(true).write(true).open(disk)?;

    let disk_size = file.seek(std::io::SeekFrom::End(0))?;

    file.seek(std::io::SeekFrom::Start(0))?;
    file.write_all(&vec![0_u8; usize::try_from(10 * MB).unwrap_or(0)])?;

    if disk_size > MB {
        file.seek(std::io::SeekFrom::Start(disk_size.saturating_sub(MB)))?;
        file.write_all(&vec![0_u8; usize::try_from(MB).unwrap_or(0)])?;
    }

    file.sync_all()?;

    Ok(())
}
