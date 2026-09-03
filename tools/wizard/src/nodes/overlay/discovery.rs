//! Overlay asset discovery from the OCI metadata listing.

use koci::pull;

use crate::domain::overlay::{self, Asset};
use crate::domain::resolution::Overlay;
use crate::error::{Result, WizardError};

/// Discovers every overlay asset from the OCI metadata listing.
///
/// # Errors
///
/// Returns an error when the OCI metadata listing fails or an entry
/// references a malformed placement.
pub(crate) fn assets(overlay: &Overlay) -> Result<Vec<Asset>> {
    let mut entries: Vec<(String, u64)> = Vec::new();
    pull::metadata(&overlay.source, &overlay.arch, None, |entry| {
        entries.push((entry.path.clone(), entry.size));

        Ok(())
    })
    .map_err(|e| WizardError::BuildError(format!("list overlay files: {e}")))?;

    overlay::classify(overlay, entries)
}
