#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::process::Command;

    fn wizard_bin() -> PathBuf {
        PathBuf::from(env!("CARGO_BIN_EXE_wizard"))
    }

    const PROFILE_TOML: &[u8] =
        b"[kernel]\nsource = \"muak-os/kernel\"\n[customization]\nextensions = []";

    #[test]
    fn cli_help_exits_successfully() {
        // ACT
        let process_output = Command::new(wizard_bin())
            .arg("--help")
            .output()
            .expect("failed to run muak-wizard --help");

        // ASSERT
        assert!(
            process_output.status.success(),
            "muak-wizard --help should exit successfully"
        );
    }

    #[test]
    fn cli_version_exits_successfully() {
        // ACT
        let process_output = Command::new(wizard_bin())
            .arg("--version")
            .output()
            .expect("failed to run muak-wizard --version");

        // ASSERT
        assert!(
            process_output.status.success(),
            "muak-wizard --version should exit successfully"
        );
        let stdout = String::from_utf8_lossy(&process_output.stdout);
        assert!(
            stdout.contains(env!("CARGO_PKG_VERSION")),
            "version output should contain package version"
        );
    }

    #[test]
    fn cli_profile_id_prints_hex() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("tempdir");
        let profile = dir.path().join("profile.toml");
        std::fs::write(&profile, PROFILE_TOML).expect("write profile");

        // ACT
        let process_output = Command::new(wizard_bin())
            .args([
                "profile-id",
                "--profile",
                profile.to_str().expect("profile path"),
            ])
            .output()
            .expect("failed to run muak-wizard profile-id");

        // ASSERT
        assert_eq!(process_output.status.code(), Some(0));
        let id = String::from_utf8_lossy(&process_output.stdout)
            .trim()
            .to_owned();
        assert_eq!(id.len(), 64, "profile-id should be 64 hex chars");
    }

    #[test]
    fn cli_resolve_prints_ids_and_registry_sources() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("tempdir");
        let profile = dir.path().join("profile.toml");
        std::fs::write(&profile, PROFILE_TOML).expect("write profile");

        // ACT
        let process_output = Command::new(wizard_bin())
            .args([
                "resolve",
                "--profile",
                profile.to_str().expect("profile path"),
                "--version",
                "latest",
                "--registry",
                "localhost:5000",
                "--arch",
                "amd64",
                "--platform",
                "metal",
            ])
            .output()
            .expect("failed to run muak-wizard resolve");

        // ASSERT
        assert!(
            process_output.status.success(),
            "muak-wizard resolve should exit successfully"
        );
        let stdout = String::from_utf8_lossy(&process_output.stdout);
        assert!(
            stdout.contains("profile id:"),
            "resolve should print the profile id: {stdout}"
        );
        assert!(
            stdout.contains("release id:"),
            "resolve should print the release id: {stdout}"
        );
        assert!(
            stdout.contains("resolution id:"),
            "resolve should print the resolution id: {stdout}"
        );
        assert!(
            stdout.contains("resolved installer: localhost:5000/installer:latest"),
            "resolve should honor --registry for the installer: {stdout}"
        );
        assert!(
            stdout.contains("resolved kernel: muak-os/kernel -> localhost:5000/kernel:latest"),
            "resolve should resolve the kernel against --registry: {stdout}"
        );
    }

    #[test]
    fn cli_without_subcommand_exits_with_error() {
        // ACT
        let process_output = Command::new(wizard_bin())
            .output()
            .expect("failed to run wizard without subcommand");

        // ASSERT
        assert!(!process_output.status.success());
    }

    #[test]
    fn cli_profile_id_missing_profile_exits_with_error() {
        // ACT
        let process_output = Command::new(wizard_bin())
            .args(["profile-id", "--profile", "/nonexistent/profile.toml"])
            .output()
            .expect("failed to run muak-wizard profile-id");

        // ASSERT
        assert_eq!(process_output.status.code(), Some(1));
    }
}
