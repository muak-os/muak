//! Resolution of uncommitted config swaps before switching root.

use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{self, Read as _};
use std::path::Path;

use anyhow::{Context as _, Result};
use blockdev::discover;
use disk::role::Role;
use rustix::fs::{CWD, Mode, mkdirat};
use rustix::mount::{MountFlags, UnmountFlags, mount, unmount};
use sha2::{Digest as _, Sha256};

const STATE_CONFIG: &str = "/run/state/config.toml";
const STATE_PREV: &str = "/run/state/config.prev";
const UPDATE_HASH: &str = "/run/state/update/uki.sha256";
const ESP_MOUNT: &str = "/run/esp";
const CMDLINE: &str = "/proc/cmdline";
const UPDATE_ID_PREFIX: &str = "muak.update_id=";

// Outcome of evaluating the uncommitted-swap rules.
#[derive(Debug, PartialEq, Eq)]
enum Decision {
    Skip,
    Restore,
    Commit,
}

/// Resolves an uncommitted config swap, restoring or committing it when needed.
pub(crate) fn handle(persistent: bool) {
    if !persistent {
        return;
    }

    let prev_exists = Path::new(STATE_PREV).exists();
    let marker = marker_present();
    let esp_matches = prev_exists && !marker && staged_matches_esp();

    match decide(prev_exists, marker, esp_matches) {
        Decision::Skip => {}
        Decision::Restore => restore(),
        Decision::Commit => commit(),
    }
}

fn decide(prev_exists: bool, marker: bool, esp_matches: bool) -> Decision {
    if !prev_exists || marker {
        return Decision::Skip;
    }
    if esp_matches {
        Decision::Commit
    } else {
        Decision::Restore
    }
}

fn restore() {
    match fs::rename(STATE_PREV, STATE_CONFIG) {
        Ok(()) => kmsg::info!("Restored previous config after an uncommitted update"),
        Err(error) => kmsg::warn!("Failed to restore previous config: {error}"),
    }
}

fn commit() {
    match fs::remove_file(STATE_PREV) {
        Ok(()) => kmsg::info!("Config swap already committed; discarded previous config"),
        Err(error) => kmsg::warn!("Failed to discard previous config: {error}"),
    }
}

fn marker_present() -> bool {
    match fs::read_to_string(CMDLINE) {
        Ok(cmdline) => cmdline
            .split_whitespace()
            .any(|arg| arg.starts_with(UPDATE_ID_PREFIX)),
        Err(error) => {
            kmsg::warn!("Failed to read {CMDLINE}: {error}; assuming update trial");
            true
        }
    }
}

fn staged_matches_esp() -> bool {
    let expected = match fs::read_to_string(UPDATE_HASH) {
        Ok(text) => text.trim().to_owned(),
        Err(error) => {
            if error.kind() != io::ErrorKind::NotFound {
                kmsg::warn!("Failed to read {UPDATE_HASH}: {error}");
            }
            return false;
        }
    };

    let Some(device) = discover::find_by_partname(Role::Esp.gpt_name())
        .into_iter()
        .next()
    else {
        kmsg::warn!("ESP partition not found; treating config swap as uncommitted");
        return false;
    };

    match read_esp_hash(&device) {
        Ok(actual) => actual.eq_ignore_ascii_case(&expected),
        Err(error) => {
            kmsg::warn!("Failed to verify ESP boot file: {error:#}");
            false
        }
    }
}

fn read_esp_hash(device: &str) -> Result<String> {
    create_mountpoint(ESP_MOUNT)?;
    mount(
        device,
        ESP_MOUNT,
        "vfat",
        MountFlags::RDONLY | MountFlags::NOSUID | MountFlags::NODEV | MountFlags::NOEXEC,
        None,
    )
    .with_context(|| format!("Failed to mount {device} at {ESP_MOUNT}"))?;

    let hashed = sha256(&Path::new(ESP_MOUNT).join(esp::arch::Arch::current().boot_path()));
    if let Err(error) = unmount(ESP_MOUNT, UnmountFlags::empty()) {
        kmsg::warn!("Failed to unmount {ESP_MOUNT}: {error}");
    }
    hashed
}

fn create_mountpoint(target: &str) -> Result<()> {
    let path = Path::new(target);
    if path.exists() {
        return Ok(());
    }
    mkdirat(CWD, path, Mode::from_bits_truncate(0o755))
        .with_context(|| format!("Failed to create {target}"))
}

fn sha256(path: &Path) -> Result<String> {
    let mut file =
        File::open(path).with_context(|| format!("Failed to open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];

    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        if read == 0 {
            break;
        }
        let chunk = buffer.get(..read).context("Failed to slice read buffer")?;
        hasher.update(chunk);
    }

    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in &digest {
        write!(&mut hex, "{byte:02x}").context("Failed to format digest")?;
    }

    Ok(hex)
}

#[cfg(test)]
mod tests {
    use super::{Decision, decide};

    #[test]
    fn decide_skips_without_previous_config() {
        // ARRANGE
        let prev_exists = false;
        let marker = false;
        let esp_matches = true;

        // ACT
        let decision = decide(prev_exists, marker, esp_matches);

        // ASSERT
        assert_eq!(decision, Decision::Skip);
    }

    #[test]
    fn decide_skips_while_update_trial_is_marked() {
        // ARRANGE
        let prev_exists = true;
        let marker = true;
        let esp_matches = false;

        // ACT
        let decision = decide(prev_exists, marker, esp_matches);

        // ASSERT
        assert_eq!(decision, Decision::Skip);
    }

    #[test]
    fn decide_skips_without_previous_config_even_when_marked() {
        // ARRANGE
        let prev_exists = false;
        let marker = true;
        let esp_matches = false;

        // ACT
        let decision = decide(prev_exists, marker, esp_matches);

        // ASSERT
        assert_eq!(decision, Decision::Skip);
    }

    #[test]
    fn decide_restores_when_esp_still_holds_the_old_boot_file() {
        // ARRANGE
        let prev_exists = true;
        let marker = false;
        let esp_matches = false;

        // ACT
        let decision = decide(prev_exists, marker, esp_matches);

        // ASSERT
        assert_eq!(decision, Decision::Restore);
    }

    #[test]
    fn decide_commits_when_esp_already_holds_the_staged_boot_file() {
        // ARRANGE
        let prev_exists = true;
        let marker = false;
        let esp_matches = true;

        // ACT
        let decision = decide(prev_exists, marker, esp_matches);

        // ASSERT
        assert_eq!(decision, Decision::Commit);
    }
}
