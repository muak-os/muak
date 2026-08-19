//! CLI integration tests for the yuki binary.

mod fixtures;

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use tempfile::TempDir;
    use yuki::cli;

    use super::fixtures::components::{fake_initrd, fake_kernel, sample_cmdline};
    use super::fixtures::pe::generate_minimal_stub;

    struct CliEnv {
        temp: TempDir,
    }

    impl CliEnv {
        fn new() -> Self {
            Self {
                temp: TempDir::new().expect("failed to create temp dir"),
            }
        }

        fn path(&self, name: &str) -> PathBuf {
            self.temp.path().join(name)
        }

        fn write(&self, name: &str, data: &[u8]) -> PathBuf {
            let path = self.path(name);
            fs::write(&path, data).unwrap_or_else(|e| panic!("failed to write {name}: {e}"));
            path
        }
    }

    fn yuki_bin() -> PathBuf {
        PathBuf::from(env!("CARGO_BIN_EXE_yuki"))
    }

    #[test]
    fn run_with_builds_uki_successfully() {
        // ARRANGE
        let env = CliEnv::new();
        let stub = env.write("stub.efi", &generate_minimal_stub());
        let kernel = env.write("vmlinuz", &fake_kernel(4096));
        let initrd = env.write("initrd.img", &fake_initrd(4096));
        let cmdline = env.write("cmdline.txt", &sample_cmdline());
        let output = env.path("output.efi");

        // ACT
        let result = cli::run_with([
            "yuki",
            "--stub",
            stub.to_str().expect("stub path"),
            "--kernel",
            kernel.to_str().expect("kernel path"),
            "--initrd",
            initrd.to_str().expect("initrd path"),
            "--cmdline",
            cmdline.to_str().expect("cmdline path"),
            "--output",
            output.to_str().expect("output path"),
        ])
        .expect("build should succeed");

        // ASSERT
        assert!(result.contains("Successfully created UKI at"));
        assert!(output.exists(), "output should be written");
        assert!(
            result.contains("bytes)"),
            "result should report output size"
        );
        let output_len = fs::metadata(&output)
            .expect("output metadata should be readable")
            .len();
        assert!(
            result.contains(&format!("({output_len} bytes)")),
            "result should contain exact output size"
        );
    }

    #[test]
    fn run_with_reports_missing_stub() {
        // ARRANGE
        let env = CliEnv::new();
        let kernel = env.write("vmlinuz", &fake_kernel(1024));
        let initrd = env.write("initrd.img", &fake_initrd(1024));
        let cmdline = env.write("cmdline.txt", &sample_cmdline());
        let output = env.path("output.efi");

        // ACT
        let error = cli::run_with([
            "yuki",
            "--stub",
            env.path("missing.efi").to_str().expect("stub path"),
            "--kernel",
            kernel.to_str().expect("kernel path"),
            "--initrd",
            initrd.to_str().expect("initrd path"),
            "--cmdline",
            cmdline.to_str().expect("cmdline path"),
            "--output",
            output.to_str().expect("output path"),
        ])
        .expect_err("missing stub should error");

        // ASSERT
        assert!(error.to_string().contains("Failed to read EFI stub"));
    }

    #[test]
    fn run_with_reports_missing_kernel() {
        // ARRANGE
        let env = CliEnv::new();
        let stub = env.write("stub.efi", &generate_minimal_stub());
        let initrd = env.write("initrd.img", &fake_initrd(1024));
        let cmdline = env.write("cmdline.txt", &sample_cmdline());
        let output = env.path("output.efi");

        // ACT
        let error = cli::run_with([
            "yuki",
            "--stub",
            stub.to_str().expect("stub path"),
            "--kernel",
            env.path("missing-kernel").to_str().expect("kernel path"),
            "--initrd",
            initrd.to_str().expect("initrd path"),
            "--cmdline",
            cmdline.to_str().expect("cmdline path"),
            "--output",
            output.to_str().expect("output path"),
        ])
        .expect_err("missing kernel should error");

        // ASSERT
        assert!(error.to_string().contains("Failed to read kernel"));
    }

    #[test]
    fn run_with_reports_missing_initramfs() {
        // ARRANGE
        let env = CliEnv::new();
        let stub = env.write("stub.efi", &generate_minimal_stub());
        let kernel = env.write("vmlinuz", &fake_kernel(1024));
        let cmdline = env.write("cmdline.txt", &sample_cmdline());
        let output = env.path("output.efi");

        // ACT
        let error = cli::run_with([
            "yuki",
            "--stub",
            stub.to_str().expect("stub path"),
            "--kernel",
            kernel.to_str().expect("kernel path"),
            "--initrd",
            env.path("missing-initrd").to_str().expect("initrd path"),
            "--cmdline",
            cmdline.to_str().expect("cmdline path"),
            "--output",
            output.to_str().expect("output path"),
        ])
        .expect_err("missing initramfs should error");

        // ASSERT
        assert!(error.to_string().contains("Failed to read initramfs"));
    }

    #[test]
    fn run_with_reports_missing_cmdline() {
        // ARRANGE
        let env = CliEnv::new();
        let stub = env.write("stub.efi", &generate_minimal_stub());
        let kernel = env.write("vmlinuz", &fake_kernel(1024));
        let initrd = env.write("initrd.img", &fake_initrd(1024));
        let output = env.path("output.efi");

        // ACT
        let error = cli::run_with([
            "yuki",
            "--stub",
            stub.to_str().expect("stub path"),
            "--kernel",
            kernel.to_str().expect("kernel path"),
            "--initrd",
            initrd.to_str().expect("initrd path"),
            "--cmdline",
            env.path("missing-cmdline").to_str().expect("cmdline path"),
            "--output",
            output.to_str().expect("output path"),
        ])
        .expect_err("missing cmdline should error");

        // ASSERT
        assert!(error.to_string().contains("Failed to read cmdline"));
    }

    #[test]
    fn run_with_reports_unwritable_output() {
        // ARRANGE
        let env = CliEnv::new();
        let stub = env.write("stub.efi", &generate_minimal_stub());
        let kernel = env.write("vmlinuz", &fake_kernel(1024));
        let initrd = env.write("initrd.img", &fake_initrd(1024));
        let cmdline = env.write("cmdline.txt", &sample_cmdline());

        // ACT
        let error = cli::run_with([
            "yuki",
            "--stub",
            stub.to_str().expect("stub path"),
            "--kernel",
            kernel.to_str().expect("kernel path"),
            "--initrd",
            initrd.to_str().expect("initrd path"),
            "--cmdline",
            cmdline.to_str().expect("cmdline path"),
            "--output",
            "/nonexistent_dir/output.efi",
        ])
        .expect_err("unwritable output should error");

        // ASSERT
        assert!(error.to_string().contains("Failed to write UKI"));
    }

    #[test]
    fn run_with_reports_build_failure() {
        // ARRANGE
        let env = CliEnv::new();
        let stub = env.write("stub.efi", b"not-a-pe");
        let kernel = env.write("vmlinuz", &fake_kernel(1024));
        let initrd = env.write("initrd.img", &fake_initrd(1024));
        let cmdline = env.write("cmdline.txt", &sample_cmdline());
        let output = env.path("output.efi");

        // ACT
        let error = cli::run_with([
            "yuki",
            "--stub",
            stub.to_str().expect("stub path"),
            "--kernel",
            kernel.to_str().expect("kernel path"),
            "--initrd",
            initrd.to_str().expect("initrd path"),
            "--cmdline",
            cmdline.to_str().expect("cmdline path"),
            "--output",
            output.to_str().expect("output path"),
        ])
        .expect_err("invalid stub should fail build");

        // ASSERT
        assert!(error.to_string().contains("Failed to compute UKI layout"));
    }

    #[test]
    fn cli_builds_uki_successfully() {
        // ARRANGE
        let env = CliEnv::new();
        let stub = env.write("stub.efi", &generate_minimal_stub());
        let kernel = env.write("vmlinuz", &fake_kernel(4096));
        let initrd = env.write("initrd.img", &fake_initrd(4096));
        let cmdline = env.write("cmdline.txt", &sample_cmdline());
        let output = env.path("output.efi");

        // ACT
        let status = std::process::Command::new(yuki_bin())
            .args([
                "--stub",
                stub.to_str().expect("stub path"),
                "--kernel",
                kernel.to_str().expect("kernel path"),
                "--initrd",
                initrd.to_str().expect("initrd path"),
                "--cmdline",
                cmdline.to_str().expect("cmdline path"),
                "--output",
                output.to_str().expect("output path"),
            ])
            .status()
            .expect("failed to run yuki");

        // ASSERT
        assert!(status.success(), "yuki should exit successfully");
        assert!(output.exists(), "output file should exist");
        let data = fs::read(&output).expect("output should be readable");
        assert!(data.starts_with(b"MZ"), "output should be a PE file");
    }

    #[test]
    fn cli_exits_with_error_on_missing_stub() {
        // ARRANGE
        let env = CliEnv::new();
        let kernel = env.write("vmlinuz", &fake_kernel(1024));
        let initrd = env.write("initrd.img", &fake_initrd(1024));
        let cmdline = env.write("cmdline.txt", &sample_cmdline());
        let output = env.path("output.efi");

        // ACT
        let status = std::process::Command::new(yuki_bin())
            .args([
                "--stub",
                env.path("nonexistent.efi").to_str().expect("stub path"),
                "--kernel",
                kernel.to_str().expect("kernel path"),
                "--initrd",
                initrd.to_str().expect("initrd path"),
                "--cmdline",
                cmdline.to_str().expect("cmdline path"),
                "--output",
                output.to_str().expect("output path"),
            ])
            .status()
            .expect("failed to run yuki");

        // ASSERT
        assert!(!status.success(), "yuki should fail with missing stub");
    }

    #[test]
    fn cli_help_succeeds() {
        // ARRANGE & ACT
        let status = std::process::Command::new(yuki_bin())
            .arg("--help")
            .status()
            .expect("failed to run yuki --help");

        // ASSERT
        assert!(status.success(), "yuki --help should exit successfully");
    }

    #[test]
    fn cli_version_succeeds() {
        // ARRANGE & ACT
        let status = std::process::Command::new(yuki_bin())
            .arg("--version")
            .status()
            .expect("failed to run yuki --version");

        // ASSERT
        assert!(status.success(), "yuki --version should exit successfully");
    }

    #[test]
    fn cli_help_contains_expected_flags() {
        // ARRANGE & ACT
        let output = std::process::Command::new(yuki_bin())
            .arg("--help")
            .output()
            .expect("failed to run yuki --help");

        // ASSERT
        let help_text = String::from_utf8_lossy(&output.stdout);
        let stderr_text = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{help_text}{stderr_text}");
        assert!(combined.contains("--stub"));
        assert!(combined.contains("--kernel"));
        assert!(combined.contains("--initrd"));
        assert!(combined.contains("--cmdline"));
        assert!(combined.contains("--output"));
    }

    #[test]
    fn run_with_reports_invalid_args() {
        // ARRANGE
        let env = CliEnv::new();
        let stub = env.write("stub.efi", &generate_minimal_stub());

        // ACT
        let error = cli::run_with([
            "yuki",
            "--stub",
            stub.to_str().expect("stub path"),
            "--nonexistent",
        ])
        .expect_err("invalid args should error");

        // ASSERT
        assert!(!error.to_string().is_empty());
    }

    #[test]
    fn cli_exits_with_error_on_unwritable_output() {
        // ARRANGE
        let env = CliEnv::new();
        let stub = env.write("stub.efi", &generate_minimal_stub());
        let kernel = env.write("vmlinuz", &fake_kernel(1024));
        let initrd = env.write("initrd.img", &fake_initrd(1024));
        let cmdline = env.write("cmdline.txt", &sample_cmdline());

        // ACT
        let status = std::process::Command::new(yuki_bin())
            .args([
                "--stub",
                stub.to_str().expect("stub path"),
                "--kernel",
                kernel.to_str().expect("kernel path"),
                "--initrd",
                initrd.to_str().expect("initrd path"),
                "--cmdline",
                cmdline.to_str().expect("cmdline path"),
                "--output",
                "/nonexistent_dir/output.efi",
            ])
            .status()
            .expect("failed to run yuki");

        // ASSERT
        assert!(
            !status.success(),
            "yuki should fail with unwritable output path"
        );
    }
}
