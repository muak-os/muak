//! Secure Boot hierarchy resolution and firmware operations for updates.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use sbolt::efi::enrollment;
use sbolt::keys::hierarchy::Bundle;
use sbolt::keys::storage::{load_hierarchy, save_hierarchy};

use super::SECRETS_DIR;

fn dir() -> PathBuf {
    Path::new(SECRETS_DIR).join("secureboot")
}

// Resolves the hierarchy for signing a staged update, generating one when none is persisted.
pub(super) fn resolve_hierarchy() -> Result<Bundle> {
    let dir = dir();
    if dir.exists() {
        load_hierarchy(&dir).context("Failed to load Secure Boot keys")
    } else {
        let keys = Bundle::generate("Muak").context("Failed to generate Secure Boot keys")?;
        save_hierarchy(&keys, &dir).context("Failed to save Secure Boot keys")?;

        Ok(keys)
    }
}

// Loads the persisted hierarchy needed to remove an enrolled Platform Key.
pub(super) fn hierarchy_for_removal() -> Result<Option<Bundle>> {
    let dir = dir();
    let hierarchy = if dir.exists() {
        Some(load_hierarchy(&dir).context("Failed to load Secure Boot keys for removal")?)
    } else {
        None
    };

    if hierarchy.is_none() && firmware::secureboot::system().is_ok_and(|state| state.pk_present) {
        bail!(
            "Cannot disable Secure Boot: the Platform Key is enrolled but the signing material \
             on STATE is missing, so it cannot be removed."
        );
    }

    Ok(hierarchy)
}

// Enrolls the Platform Key when the firmware does not have one.
pub(super) fn enroll_if_missing() -> Result<()> {
    let pk_present = firmware::secureboot::system()
        .context("Failed to read PK from firmware")?
        .pk_present;
    if pk_present {
        return Ok(());
    }

    let hierarchy =
        load_hierarchy(&dir()).context("Failed to load Secure Boot keys for enrollment")?;

    enrollment::enroll(&firmware::system(), &hierarchy)
        .context("Failed to enroll Secure Boot keys into firmware")
}

// Removes the enrolled Platform Key using the persisted hierarchy.
pub(super) fn unenroll_if_present() -> Result<()> {
    let pk_present = firmware::secureboot::system()
        .context("Failed to read PK from firmware")?
        .pk_present;
    if !pk_present {
        return Ok(());
    }

    let hierarchy =
        load_hierarchy(&dir()).context("Failed to load Secure Boot keys for removal")?;

    enrollment::unenroll(&firmware::system(), &hierarchy)
        .context("Failed to remove the Secure Boot Platform Key")
}

// Re-enrolls the persisted hierarchy after a rollback returns to Secure Boot.
pub(super) fn re_enroll_if_missing() -> Result<()> {
    let state = firmware::secureboot::system().context("Failed to read Secure Boot state")?;
    if state.pk_present {
        return Ok(());
    }

    let dir = dir();
    if !dir.exists() {
        return Ok(());
    }

    let hierarchy = load_hierarchy(&dir).context("Failed to load Secure Boot keys")?;

    enrollment::enroll(&firmware::system(), &hierarchy)
        .context("Failed to re-enroll Secure Boot keys")
}

// Removes the persisted signing material once enforcement is off.
pub(super) fn remove_hierarchy() {
    let dir = dir();
    if !dir.exists() {
        return;
    }

    if let Err(e) = std::fs::remove_dir_all(&dir) {
        kmsg::warn!("Failed to remove stale Secure Boot keys: {e}");
    }
}
