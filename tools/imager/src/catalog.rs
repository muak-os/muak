//! Official extension inventory and name resolution.

// TODO: Remove this module once we have a more robust extension registry and discovery mechanism in place.

/// Checked-in official extension repositories for the first factory slice.
const OFFICIAL_EXTENSION_REPOSITORIES: &[&str] = &["muak-os/qemu"];

/// Normalizes legacy extension names to canonical logical names.
#[must_use]
pub fn resolve_extension_name(name: &str) -> &str {
    match name {
        "qemu" => "muak-os/qemu",
        other => other,
    }
}

/// Derives the stable archive basename for an extension.
#[must_use]
pub fn extension_archive_name(name: &str) -> String {
    name.replace('/', "-")
}

/// Returns true when a logical extension belongs to the checked-in official inventory.
#[must_use]
pub fn is_official_extension(name: &str) -> bool {
    OFFICIAL_EXTENSION_REPOSITORIES.binary_search(&name).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_legacy_extension_names() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(resolve_extension_name("qemu"), "muak-os/qemu");
        assert_eq!(resolve_extension_name("custom"), "custom");
        assert_eq!(resolve_extension_name("muak-os/qemu"), "muak-os/qemu");
    }

    #[test]
    fn derives_stable_archive_names() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(extension_archive_name("muak-os/qemu"), "muak-os-qemu");
        assert_eq!(
            extension_archive_name("muak-os/iscsi-tools"),
            "muak-os-iscsi-tools"
        );
    }

    #[test]
    fn identifies_official_extensions() {
        // ARRANGE / ACT / ASSERT
        assert!(is_official_extension("muak-os/qemu"));
        assert!(!is_official_extension("custom/thing"));
    }
}
