//! Resolution of a pending config swap against the booted artifact version.

use std::path::Path;

use rustix::system::RebootCommand;

use super::snapshot;
use crate::medium;

// Outcome of comparing the installed config version with the booted artifact.
#[derive(Debug, PartialEq, Eq)]
enum Decision {
    Keep,
    Restore,
}

/// Keeps the installed config when it belongs to the booted artifact, restores it otherwise.
pub(crate) fn resolve_pending_config() {
    if !Path::new(snapshot::CONFIG_PREV_PATH).exists() {
        return;
    }

    let declared = installed_version();
    let artifact = medium::version().ok();

    match decide(declared.as_deref(), artifact.as_deref()) {
        Decision::Keep => discard_stale(),
        Decision::Restore => {
            log_restore(declared.as_deref(), artifact.as_deref());
            restore();
        }
    }
}

fn decide(declared: Option<&str>, artifact: Option<&str>) -> Decision {
    match (declared, artifact) {
        (Some(declared), Some(artifact)) if declared == artifact => Decision::Keep,
        _ => Decision::Restore,
    }
}

fn installed_version() -> Option<String> {
    let raw = match std::fs::read_to_string(config::system::CONFIG_PATH) {
        Ok(raw) => raw,
        Err(error) => {
            kmsg::warn!("Failed to read the installed config: {error}");
            return None;
        }
    };

    match config::document::version(&raw) {
        Ok(version) => version,
        Err(error) => {
            kmsg::warn!("Failed to lift the installed config version: {error}");
            None
        }
    }
}

fn log_restore(declared: Option<&str>, artifact: Option<&str>) {
    match (declared, artifact) {
        (Some(declared), Some(artifact)) => {
            kmsg::warn!(
                "Config version {declared} predates the booted artifact {artifact}; restoring the previous config"
            );
        }
        (None, _) => {
            kmsg::warn!("Installed config version is unreadable; restoring the previous config");
        }
        (Some(_), None) => {
            kmsg::warn!("Booted artifact version is unavailable; restoring the previous config");
        }
    }
}

fn discard_stale() {
    match std::fs::remove_file(snapshot::CONFIG_PREV_PATH) {
        Ok(()) => kmsg::info!("Previous config is stale for the booted artifact; discarded it"),
        Err(error) => kmsg::error!("Failed to discard the previous config: {error}"),
    }
}

fn restore() {
    if let Err(error) = std::fs::rename(snapshot::CONFIG_PREV_PATH, config::system::CONFIG_PATH) {
        kmsg::error!("Failed to restore the previous config: {error}");
        return;
    }

    rustix::fs::sync();
    kmsg::info!("Previous config restored; rebooting to retry the boot");

    if let Err(error) = rustix::system::reboot(RebootCommand::Restart) {
        kmsg::error!("Failed to reboot after restoring the previous config: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::{Decision, decide};

    #[test]
    fn equal_versions_keep_the_installed_config() {
        // ARRANGE
        let declared = Some("v1.2.0");
        let artifact = Some("v1.2.0");

        // ACT
        let decision = decide(declared, artifact);

        // ASSERT
        assert_eq!(decision, Decision::Keep);
    }

    #[test]
    fn different_versions_restore_the_previous_config() {
        // ARRANGE
        let declared = Some("v1.2.0");
        let artifact = Some("v1.3.0");

        // ACT
        let decision = decide(declared, artifact);

        // ASSERT
        assert_eq!(decision, Decision::Restore);
    }

    #[test]
    fn missing_declared_version_restores_the_previous_config() {
        // ARRANGE
        let declared = None;
        let artifact = Some("v1.3.0");

        // ACT
        let decision = decide(declared, artifact);

        // ASSERT
        assert_eq!(decision, Decision::Restore);
    }

    #[test]
    fn missing_artifact_version_restores_the_previous_config() {
        // ARRANGE
        let declared = Some("v1.3.0");
        let artifact = None;

        // ACT
        let decision = decide(declared, artifact);

        // ASSERT
        assert_eq!(decision, Decision::Restore);
    }

    #[test]
    fn missing_both_versions_restores_the_previous_config() {
        // ARRANGE
        let declared = None;
        let artifact = None;

        // ACT
        let decision = decide(declared, artifact);

        // ASSERT
        assert_eq!(decision, Decision::Restore);
    }
}
