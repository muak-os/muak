//! UKI (Unified Kernel Image) building.

use std::io::{Cursor, Read as _, Write};

use tokio::task::spawn_blocking;

use super::stage::InstallerAssets;
use crate::error::{WizardError, Result};

/// Writes a combined initramfs to a `Write` sink.
///
/// # Errors
///
/// Returns an error when reading the base initramfs or writing fails.
pub async fn write_initramfs_to_writer<W: Write>(
    assets: &InstallerAssets,
    tail: &[u8],
    writer: &mut W,
) -> Result<()> {
    let base_file = assets.initramfs.clone();
    let tail = tail.to_vec();
    let buf = spawn_blocking(move || {
        let base_reader = base_file
            .open()
            .map_err(|e| WizardError::BuildError(format!("open initramfs: {e}")))?;
        let tail_reader = Cursor::new(tail.as_slice());
        let mut combined = base_reader.chain(tail_reader);
        // TODO: Optimize this
        let mut buf = Vec::new();
        std::io::copy(&mut combined, &mut buf)
            .map_err(|e| WizardError::BuildError(format!("read initramfs: {e}")))?;

        Ok::<_, WizardError>(buf)
    })
    .await
    .map_err(|e| WizardError::BuildError(format!("join initramfs task: {e}")))??;

    writer
        .write_all(&buf)
        .map_err(|e| WizardError::BuildError(format!("write initramfs: {e}")))?;

    Ok(())
}
