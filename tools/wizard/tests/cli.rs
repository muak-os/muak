#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::process::Command;

    use tokio::runtime::Runtime;
    use wizard::cli;

    fn wizard_bin() -> PathBuf {
        PathBuf::from(env!("CARGO_BIN_EXE_wizard"))
    }

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
        std::fs::write(&profile, b"[customization]\nextensions = []").expect("write profile");

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
        assert!(
            process_output.status.success(),
            "muak-wizard profile-id should exit successfully"
        );
        let id = String::from_utf8_lossy(&process_output.stdout)
            .trim()
            .to_owned();
        assert_eq!(id.len(), 64, "profile-id should be 64 hex chars");
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

    #[tokio::test]
    async fn run_with_profile_id_prints_hex() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("tempdir");
        let profile = dir.path().join("profile.toml");
        std::fs::write(&profile, b"[customization]\nextensions = []").expect("write profile");

        // ACT
        cli::run_from([
            "muak-wizard",
            "profile-id",
            "--profile",
            profile.to_str().expect("profile path"),
        ])
        .await
        .expect("run_from profile-id");
    }

    #[tokio::test]
    async fn run_with_returns_zero_for_success() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("tempdir");
        let profile = dir.path().join("profile.toml");
        std::fs::write(&profile, b"[customization]\nextensions = []").expect("write profile");

        // ACT
        let exit_code = cli::run_with([
            "muak-wizard",
            "profile-id",
            "--profile",
            profile.to_str().expect("profile path"),
        ])
        .await;

        // ASSERT
        assert_eq!(exit_code, 0);
    }

    #[test]
    fn run_with_returns_one_for_error() {
        // ACT
        let exit_code = Runtime::new().expect("runtime").block_on(cli::run_with([
            "muak-wizard",
            "profile-id",
            "--profile",
            "/nonexistent/profile.toml",
        ]));

        // ASSERT
        assert_eq!(exit_code, 1);
    }
}
