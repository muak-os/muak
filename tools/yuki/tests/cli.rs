//! CLI integration tests for the yuki binary.

mod fixtures;

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use tempfile::TempDir;
    use yuki::cli;

    use super::fixtures::components::{fake_dtb, fake_initrd, fake_kernel, sample_cmdline};
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
            "--linux",
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
    fn run_with_reads_optional_dtb() {
        // ARRANGE
        let env = CliEnv::new();
        let stub = env.write("stub.efi", &generate_minimal_stub());
        let kernel = env.write("vmlinuz", &fake_kernel(4096));
        let initrd = env.write("initrd.img", &fake_initrd(4096));
        let cmdline = env.write("cmdline.txt", &sample_cmdline());
        let dtb = env.write("device.dtb", &fake_dtb(1024));
        let output = env.path("output.efi");

        // ACT
        cli::run_with([
            "yuki",
            "--stub",
            stub.to_str().expect("stub path"),
            "--linux",
            kernel.to_str().expect("kernel path"),
            "--initrd",
            initrd.to_str().expect("initrd path"),
            "--cmdline",
            cmdline.to_str().expect("cmdline path"),
            "--dtb",
            dtb.to_str().expect("dtb path"),
            "--output",
            output.to_str().expect("output path"),
        ])
        .expect("build with optional dtb should succeed");

        // ASSERT
        assert!(output.exists(), "output should be written");
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
            "--linux",
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
            "--linux",
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
            "--linux",
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
            "--linux",
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
    fn run_with_reports_missing_dtb() {
        // ARRANGE
        let env = CliEnv::new();
        let stub = env.write("stub.efi", &generate_minimal_stub());
        let kernel = env.write("vmlinuz", &fake_kernel(1024));
        let initrd = env.write("initrd.img", &fake_initrd(1024));
        let cmdline = env.write("cmdline.txt", &sample_cmdline());
        let output = env.path("output.efi");

        // ACT
        let error = cli::run_with([
            "yuki",
            "--stub",
            stub.to_str().expect("stub path"),
            "--linux",
            kernel.to_str().expect("kernel path"),
            "--initrd",
            initrd.to_str().expect("initrd path"),
            "--cmdline",
            cmdline.to_str().expect("cmdline path"),
            "--dtb",
            env.path("missing.dtb").to_str().expect("dtb path"),
            "--output",
            output.to_str().expect("output path"),
        ])
        .expect_err("missing dtb should error");

        // ASSERT
        assert!(error.to_string().contains("Failed to read DTB"));
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
            "--linux",
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
            "--linux",
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
                "--linux",
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
    fn cli_builds_uki_with_dtb() {
        // ARRANGE
        let env = CliEnv::new();
        let stub = env.write("stub.efi", &generate_minimal_stub());
        let kernel = env.write("vmlinuz", &fake_kernel(4096));
        let initrd = env.write("initrd.img", &fake_initrd(4096));
        let cmdline = env.write("cmdline.txt", &sample_cmdline());
        let dtb = env.write("device.dtb", &fake_dtb(1024));
        let output = env.path("output.efi");

        // ACT
        let status = std::process::Command::new(yuki_bin())
            .args([
                "--stub",
                stub.to_str().expect("stub path"),
                "--linux",
                kernel.to_str().expect("kernel path"),
                "--initrd",
                initrd.to_str().expect("initrd path"),
                "--cmdline",
                cmdline.to_str().expect("cmdline path"),
                "--dtb",
                dtb.to_str().expect("dtb path"),
                "--output",
                output.to_str().expect("output path"),
            ])
            .status()
            .expect("failed to run yuki");

        // ASSERT
        assert!(status.success(), "yuki with dtb should succeed");
        assert!(output.exists(), "output file should exist");
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
                "--linux",
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
                "--linux",
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
