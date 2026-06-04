#[cfg(test)]
mod fixtures;

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    use ramune::cli;

    use super::fixtures::{TestEnv, decode_extension_archive, decode_initramfs};

    fn ramune_bin() -> PathBuf {
        PathBuf::from(env!("CARGO_BIN_EXE_ramune"))
    }

    #[test]
    fn cli_create_builds_initramfs() {
        // ARRANGE
        let env = TestEnv::new();
        let init = env.write("init", b"#!/bin/sh\nexec /sbin/init\n");
        let rootfs = env.write_rootfs();
        let output = env.path("initramfs.img");

        // ACT
        let process_output = Command::new(ramune_bin())
            .args([
                "create",
                "--init",
                init.to_str().expect("init path"),
                "--rootfs-dir",
                rootfs.to_str().expect("rootfs path"),
                "--output",
                output.to_str().expect("output path"),
            ])
            .output()
            .expect("failed to run ramune create");

        // ASSERT
        assert!(
            process_output.status.success(),
            "ramune create should exit successfully"
        );
        assert!(
            String::from_utf8_lossy(&process_output.stdout)
                .contains("Successfully created initramfs at")
        );
        let entries = decode_initramfs(&output);
        let init_entry = entries
            .iter()
            .find(|entry| entry.name == "init")
            .expect("missing init entry");
        assert_eq!(init_entry.mode, 0o100_755);
        assert_eq!(init_entry.data, b"#!/bin/sh\nexec /sbin/init\n");

        let rootfs_entry = entries
            .iter()
            .find(|entry| entry.name == "rootfs.erofs")
            .expect("missing rootfs erofs entry");
        assert_eq!(rootfs_entry.mode, 0o100_644);
        assert!(
            !rootfs_entry.data.is_empty(),
            "rootfs erofs should not be empty"
        );
    }

    #[test]
    fn cli_create_with_file_contexts_builds_initramfs() {
        // ARRANGE
        let env = TestEnv::new();
        let init = env.write("init", b"#!/bin/sh\nexec /sbin/init\n");
        let rootfs = env.write_rootfs();
        let file_contexts = env.write("file_contexts", b"/.*    system_u:object_r:file_t:s0\n");
        let output = env.path("initramfs.img");

        // ACT
        let process_output = Command::new(ramune_bin())
            .args([
                "create",
                "--init",
                init.to_str().expect("init path"),
                "--rootfs-dir",
                rootfs.to_str().expect("rootfs path"),
                "--file-contexts",
                file_contexts.to_str().expect("file contexts path"),
                "--output",
                output.to_str().expect("output path"),
            ])
            .output()
            .expect("failed to run ramune create with file contexts");

        // ASSERT
        assert!(
            process_output.status.success(),
            "ramune create should accept file contexts"
        );
        assert!(
            String::from_utf8_lossy(&process_output.stdout)
                .contains("Successfully created initramfs at")
        );
        assert!(output.exists(), "output image should exist");
    }

    #[test]
    fn cli_create_accepts_separate_rootfs_compression_level() {
        // ARRANGE
        let env = TestEnv::new();
        let init = env.write("init", b"#!/bin/sh\nexec /sbin/init\n");
        let rootfs = env.write_rootfs();
        let output = env.path("initramfs.img");

        // ACT
        let process_output = Command::new(ramune_bin())
            .args([
                "create",
                "--init",
                init.to_str().expect("init path"),
                "--rootfs-dir",
                rootfs.to_str().expect("rootfs path"),
                "--compression-level",
                "19",
                "--rootfs-compression-level",
                "7",
                "--output",
                output.to_str().expect("output path"),
            ])
            .output()
            .expect("failed to run ramune create with separate compression levels");

        // ASSERT
        assert!(
            process_output.status.success(),
            "ramune create should accept separate rootfs compression level"
        );
        assert!(output.exists(), "output image should exist");
    }

    #[test]
    fn cli_create_invalid_rootfs_compression_level_exits_with_error() {
        // ARRANGE
        let env = TestEnv::new();
        let init = env.write("init", b"#!/bin/sh\nexec /sbin/init\n");
        let rootfs = env.write_rootfs();
        let output = env.path("initramfs.img");

        // ACT
        let process_output = Command::new(ramune_bin())
            .args([
                "create",
                "--init",
                init.to_str().expect("init path"),
                "--rootfs-dir",
                rootfs.to_str().expect("rootfs path"),
                "--rootfs-compression-level",
                &i32::MAX.to_string(),
                "--output",
                output.to_str().expect("output path"),
            ])
            .output()
            .expect("failed to run ramune create with invalid rootfs compression level");

        // ASSERT
        assert!(
            !process_output.status.success(),
            "ramune create should fail for invalid rootfs compression level"
        );
        assert!(
            String::from_utf8_lossy(&process_output.stderr).contains("Invalid compression level")
        );
    }

    #[test]
    fn cli_create_missing_init_exits_with_error() {
        // ARRANGE
        let env = TestEnv::new();
        let rootfs = env.write_rootfs();
        let output = env.path("initramfs.img");

        // ACT
        let process_output = Command::new(ramune_bin())
            .args([
                "create",
                "--init",
                env.path("missing-init").to_str().expect("init path"),
                "--rootfs-dir",
                rootfs.to_str().expect("rootfs path"),
                "--output",
                output.to_str().expect("output path"),
            ])
            .output()
            .expect("failed to run ramune create");

        // ASSERT
        assert!(
            !process_output.status.success(),
            "ramune create should fail for missing init"
        );
        assert!(
            String::from_utf8_lossy(&process_output.stderr).contains("Failed to create initramfs")
        );
    }

    #[test]
    fn cli_help_exits_successfully() {
        // ACT
        let process_output = Command::new(ramune_bin())
            .arg("--help")
            .output()
            .expect("failed to run ramune --help");

        // ASSERT
        assert!(
            process_output.status.success(),
            "ramune --help should exit successfully"
        );
        let stdout = String::from_utf8_lossy(&process_output.stdout);
        assert!(stdout.contains("Usage: ramune <COMMAND>"));
        assert!(
            process_output.stderr.is_empty(),
            "help should not print to stderr"
        );
    }

    #[test]
    fn cli_version_exits_successfully() {
        // ACT
        let process_output = Command::new(ramune_bin())
            .arg("--version")
            .output()
            .expect("failed to run ramune --version");

        // ASSERT
        assert!(
            process_output.status.success(),
            "ramune --version should exit successfully"
        );
        let stdout = String::from_utf8_lossy(&process_output.stdout);
        assert!(stdout.contains(env!("CARGO_PKG_VERSION")));
        assert!(
            process_output.stderr.is_empty(),
            "version should not print to stderr"
        );
    }

    #[test]
    fn cli_extend_builds_extended_initramfs() {
        // ARRANGE
        let env = TestEnv::new();
        let base_bytes = b"base-initramfs";
        let base = env.write("base.img", base_bytes);
        let extension = env.write_extension("cli-ext", b"payload");
        let output = env.path("extended.img");

        // ACT
        let process_output = Command::new(ramune_bin())
            .args([
                "extend",
                "--base",
                base.to_str().expect("base path"),
                "--extra",
                &format!(
                    "{}:extensions/cli-ext.erofs:true",
                    extension.to_str().expect("extension path")
                ),
                "--output",
                output.to_str().expect("output path"),
            ])
            .output()
            .expect("failed to run ramune extend");

        // ASSERT
        assert!(
            process_output.status.success(),
            "ramune extend should exit successfully"
        );
        assert!(
            String::from_utf8_lossy(&process_output.stdout)
                .contains("Successfully created initramfs at")
        );
        let image = fs::read(&output).expect("read output");
        assert!(image.starts_with(base_bytes));

        let entries = decode_extension_archive(&output, base_bytes.len());
        let entry = entries
            .iter()
            .find(|entry| entry.name == "extensions/cli-ext.erofs")
            .expect("missing extension entry");
        assert_eq!(entry.mode, 0o100_644);
        assert!(
            !entry.data.is_empty(),
            "extension erofs should not be empty"
        );
    }

    #[test]
    fn cli_extend_plain_file() {
        // ARRANGE
        let env = TestEnv::new();
        let base_bytes = b"base-initramfs";
        let base = env.write("base.img", base_bytes);
        let profile = env.write("profile.toml", b"[overlay]\nname = \"test\"\n");
        let output = env.path("extended.img");

        // ACT
        let process_output = Command::new(ramune_bin())
            .args([
                "extend",
                "--base",
                base.to_str().expect("base path"),
                "--extra",
                &format!("{}:profile.toml", profile.to_str().expect("profile path")),
                "--output",
                output.to_str().expect("output path"),
            ])
            .output()
            .expect("failed to run ramune extend");

        // ASSERT
        assert!(
            process_output.status.success(),
            "ramune extend with plain file should exit successfully"
        );
        let image = fs::read(&output).expect("read output");
        assert!(image.starts_with(base_bytes));

        let entries = decode_extension_archive(&output, base_bytes.len());
        let entry = entries
            .iter()
            .find(|entry| entry.name == "profile.toml")
            .expect("missing profile entry");
        assert_eq!(entry.data, b"[overlay]\nname = \"test\"\n");
    }

    #[test]
    fn cli_extend_invalid_compression_level_exits_with_error() {
        // ARRANGE
        let env = TestEnv::new();
        let base = env.write("base.img", b"base-initramfs");
        let extension = env.write_extension("cli-ext", b"payload");
        let output = env.path("extended.img");

        // ACT
        let process_output = Command::new(ramune_bin())
            .args([
                "extend",
                "--base",
                base.to_str().expect("base path"),
                "--extra",
                &format!(
                    "{}:extensions/cli-ext.erofs:true",
                    extension.to_str().expect("extension path")
                ),
                "--compression-level",
                &i32::MAX.to_string(),
                "--output",
                output.to_str().expect("output path"),
            ])
            .output()
            .expect("failed to run ramune extend with invalid compression level");

        // ASSERT
        assert!(
            !process_output.status.success(),
            "ramune extend should fail for invalid compression level"
        );
        assert!(
            String::from_utf8_lossy(&process_output.stderr).contains("Invalid compression level")
        );
    }

    #[test]
    fn cli_extend_missing_base_exits_with_error() {
        // ARRANGE
        let env = TestEnv::new();
        let output = env.path("extended.img");

        // ACT
        let process_output = Command::new(ramune_bin())
            .args([
                "extend",
                "--base",
                env.path("missing-base").to_str().expect("base path"),
                "--output",
                output.to_str().expect("output path"),
            ])
            .output()
            .expect("failed to run ramune extend");

        // ASSERT
        assert!(
            !process_output.status.success(),
            "ramune extend should fail for missing base"
        );
        assert!(
            String::from_utf8_lossy(&process_output.stderr).contains("Failed to build initramfs")
        );
    }

    #[test]
    fn cli_extend_malformed_extra_file_errors() {
        // ARRANGE
        let env = TestEnv::new();
        let base = env.write("base.img", b"base-initramfs");
        let output = env.path("extended.img");

        // ACT
        let process_output = Command::new(ramune_bin())
            .args([
                "extend",
                "--base",
                base.to_str().expect("base path"),
                "--extra",
                "/tmp/src",
                "--output",
                output.to_str().expect("output path"),
            ])
            .output()
            .expect("failed to run ramune extend");

        // ASSERT
        assert!(
            !process_output.status.success(),
            "ramune extend should fail for malformed extra file"
        );
    }

    #[tokio::test]
    async fn run_with_create_writes_output() {
        // ARRANGE
        let env = TestEnv::new();
        let init = env.write("init", b"#!/bin/sh\nexec /sbin/init\n");
        let rootfs = env.write_rootfs();
        let output = env.path("run-with-initramfs.img");

        // ACT
        cli::run_from([
            "ramune",
            "create",
            "--init",
            init.to_str().expect("init path"),
            "--rootfs-dir",
            rootfs.to_str().expect("rootfs path"),
            "--output",
            output.to_str().expect("output path"),
        ])
        .await
        .expect("run_from create");

        // ASSERT
        assert!(output.exists());
    }

    #[tokio::test]
    async fn run_with_create_with_file_contexts_writes_output() {
        // ARRANGE
        let env = TestEnv::new();
        let init = env.write("init", b"#!/bin/sh\nexec /sbin/init\n");
        let rootfs = env.write_rootfs();
        let file_contexts = env.write("file_contexts", b"/.*    system_u:object_r:file_t:s0\n");
        let output = env.path("run-with-initramfs.img");

        // ACT
        cli::run_from([
            "ramune",
            "create",
            "--init",
            init.to_str().expect("init path"),
            "--rootfs-dir",
            rootfs.to_str().expect("rootfs path"),
            "--file-contexts",
            file_contexts.to_str().expect("file_contexts path"),
            "--output",
            output.to_str().expect("output path"),
        ])
        .await
        .expect("run_from create");

        // ASSERT
        assert!(output.exists());
    }

    #[tokio::test]
    async fn run_with_create_accepts_rootfs_compression_level() {
        // ARRANGE
        let env = TestEnv::new();
        let init = env.write("init", b"#!/bin/sh\nexec /sbin/init\n");
        let rootfs = env.write_rootfs();
        let output = env.path("run-with-initramfs.img");

        // ACT
        cli::run_from([
            "ramune",
            "create",
            "--init",
            init.to_str().expect("init path"),
            "--rootfs-dir",
            rootfs.to_str().expect("rootfs path"),
            "--rootfs-compression-level",
            "7",
            "--output",
            output.to_str().expect("output path"),
        ])
        .await
        .expect("run_from create");

        // ASSERT
        assert!(output.exists());
    }

    #[tokio::test]
    async fn run_with_create_missing_file_contexts_errors() {
        // ARRANGE
        let env = TestEnv::new();
        let init = env.write("init", b"#!/bin/sh\nexec /sbin/init\n");
        let rootfs = env.write_rootfs();
        let output = env.path("run-with-initramfs.img");

        // ACT
        let result = cli::run_from([
            "ramune",
            "create",
            "--init",
            init.to_str().expect("init path"),
            "--rootfs-dir",
            rootfs.to_str().expect("rootfs path"),
            "--file-contexts",
            env.path("missing-file-contexts")
                .to_str()
                .expect("file_contexts path"),
            "--output",
            output.to_str().expect("output path"),
        ])
        .await;

        // ASSERT
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn run_with_create_invalid_file_contexts_errors() {
        // ARRANGE
        let env = TestEnv::new();
        let init = env.write("init", b"#!/bin/sh\nexec /sbin/init\n");
        let rootfs = env.write_rootfs();
        let file_contexts = env.write("file_contexts", b"/path -- ctx extra\n");
        let output = env.path("run-with-initramfs.img");

        // ACT
        let result = cli::run_from([
            "ramune",
            "create",
            "--init",
            init.to_str().expect("init path"),
            "--rootfs-dir",
            rootfs.to_str().expect("rootfs path"),
            "--file-contexts",
            file_contexts.to_str().expect("file_contexts path"),
            "--output",
            output.to_str().expect("output path"),
        ])
        .await;

        // ASSERT
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn run_with_extend_writes_output() {
        // ARRANGE
        let env = TestEnv::new();
        let base = env.write("base.img", b"base");
        let extension = env.write_extension("run-with-ext", b"payload");
        let output = env.path("run-with-extended.img");

        // ACT
        cli::run_from([
            "ramune",
            "extend",
            "--base",
            base.to_str().expect("base path"),
            "--extra",
            &format!(
                "{}:extensions/run-with-ext.erofs:true",
                extension.to_str().expect("extension path")
            ),
            "--output",
            output.to_str().expect("output path"),
        ])
        .await
        .expect("run_from extend");

        // ASSERT
        assert!(output.exists());
    }

    #[tokio::test]
    async fn run_with_extend_missing_base_errors() {
        // ARRANGE
        let env = TestEnv::new();
        let output = env.path("run-with-extended.img");

        // ACT
        let result = cli::run_from([
            "ramune",
            "extend",
            "--base",
            env.path("missing.img").to_str().expect("base path"),
            "--output",
            output.to_str().expect("output path"),
        ])
        .await;

        // ASSERT
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn run_with_returns_zero_for_success() {
        // ARRANGE
        let env = TestEnv::new();
        let init = env.write("init", b"#!/bin/sh\nexec /sbin/init\n");
        let rootfs = env.write_rootfs();
        let output = env.path("run-with-initramfs.img");

        // ACT
        let exit_code = cli::run_with([
            "ramune",
            "create",
            "--init",
            init.to_str().expect("init path"),
            "--rootfs-dir",
            rootfs.to_str().expect("rootfs path"),
            "--output",
            output.to_str().expect("output path"),
        ])
        .await;

        // ASSERT
        assert_eq!(exit_code, 0);
    }

    #[tokio::test]
    async fn run_with_returns_one_for_error() {
        // ARRANGE
        let env = TestEnv::new();
        let output = env.path("run-with-extended.img");

        // ACT
        let exit_code = cli::run_with([
            "ramune",
            "extend",
            "--base",
            env.path("missing.img").to_str().expect("base path"),
            "--output",
            output.to_str().expect("output path"),
        ])
        .await;

        // ASSERT
        assert_eq!(exit_code, 1);
    }

    #[test]
    fn cli_without_subcommand_exits_with_error() {
        // ACT
        let process_output = Command::new(ramune_bin())
            .output()
            .expect("failed to run ramune without subcommand");

        // ASSERT
        assert!(!process_output.status.success());
        assert!(
            String::from_utf8_lossy(&process_output.stderr).contains("Usage: ramune <COMMAND>")
        );
    }
}
