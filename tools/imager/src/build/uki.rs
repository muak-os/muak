//! UKI (Unified Kernel Image) building.

use std::io::{Cursor, Read as _};
use std::path::Path;

use tokio::task::spawn_blocking;

use super::stage::InstallerAssets;
use crate::error::{ImagerError, Result};

/// Writes a combined (base + tail) initramfs to a file on disk.
///
/// # Errors
///
/// Returns an error when reading the base initramfs or writing the file fails.
pub async fn write_initramfs(assets: &InstallerAssets, tail: &[u8], output: &Path) -> Result<()> {
    let base_file = assets.initramfs.clone();
    let tail = tail.to_vec();
    let output = output.to_path_buf();
    spawn_blocking(move || {
        let base_reader = base_file
            .open()
            .map_err(|e| ImagerError::BuildError(format!("open initramfs: {e}")))?;
        let tail_reader = Cursor::new(tail.as_slice());
        let mut combined = base_reader.chain(tail_reader);
        let file = std::fs::File::create(&output)
            .map_err(|e| ImagerError::BuildError(format!("create initramfs file: {e}")))?;
        let mut writer = std::io::BufWriter::new(file);
        std::io::copy(&mut combined, &mut writer)
            .map_err(|e| ImagerError::BuildError(format!("write initramfs: {e}")))?;
        Ok::<_, ImagerError>(())
    })
    .await
    .map_err(|e| ImagerError::BuildError(format!("join write initramfs task: {e}")))?
}
