#[cfg(test)]
mod fixtures;

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    use ramune::cli;

    use super::fixtures::{TestEnv, decode_initramfs, parse_newc_archive};

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
        assert!(process_output.status.success());
        assert!(
            String::from_utf8_lossy(&process_output.stdout)
                .contains("Successfully created initramfs at")
        );
        let entries = decode_initramfs(&output);
        assert!(entries.iter().any(|entry| entry.name == "init"));
        assert!(entries.iter().any(|entry| entry.name == "rootfs.erofs"));
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
        assert!(process_output.status.success());
        assert!(output.exists());
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
        assert!(!process_output.status.success());
        assert!(
            String::from_utf8_lossy(&process_output.stderr).contains("Failed to read init binary")
        );
    }

    #[test]
    fn cli_tail_builds_compressed_archive() {
        // ARRANGE
        let env = TestEnv::new();
        let profile = env.write("profile.toml", b"[overlay]\nname = \"test\"\n");
        let extension = env.write("cli-ext.erofs", b"payload");
        let output = env.path("tail.img");

        // ACT
        let process_output = Command::new(ramune_bin())
            .args([
                "tail",
                "--entry",
                &format!("{}:profile.toml", profile.to_str().expect("profile path")),
                "--entry",
                &format!(
                    "{}:extensions/cli-ext.erofs",
                    extension.to_str().expect("extension path")
                ),
                "--output",
                output.to_str().expect("output path"),
            ])
            .output()
            .expect("failed to run ramune tail");

        // ASSERT
        assert!(process_output.status.success());
        assert!(
            String::from_utf8_lossy(&process_output.stdout)
                .contains("Successfully created initramfs tail at")
        );

        let archive = zstd::decode_all(fs::read(&output).expect("read tail").as_slice())
            .expect("decode tail");
        let entries = parse_newc_archive(&archive);
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["extensions/cli-ext.erofs", "profile.toml"]);
        assert_eq!(entries.first().expect("first entry").mode, 0o100_644);
        assert_eq!(
            entries.get(1).expect("second entry").data,
            b"[overlay]\nname = \"test\"\n"
        );
    }

    #[test]
    fn cli_tail_invalid_compression_level_exits_with_error() {
        // ARRANGE
        let env = TestEnv::new();
        let profile = env.write("profile.toml", b"[overlay]\nname = \"test\"\n");
        let output = env.path("tail.img");

        // ACT
        let process_output = Command::new(ramune_bin())
            .args([
                "tail",
                "--entry",
                &format!("{}:profile.toml", profile.to_str().expect("profile path")),
                "--compression-level",
                &i32::MAX.to_string(),
                "--output",
                output.to_str().expect("output path"),
            ])
            .output()
            .expect("failed to run ramune tail");

        // ASSERT
        assert!(!process_output.status.success());
        assert!(
            String::from_utf8_lossy(&process_output.stderr).contains("Invalid compression level")
        );
    }

    #[test]
    fn cli_tail_missing_entry_source_exits_with_error() {
        // ARRANGE
        let env = TestEnv::new();
        let output = env.path("tail.img");

        // ACT
        let process_output = Command::new(ramune_bin())
            .args([
                "tail",
                "--entry",
                &format!(
                    "{}:profile.toml",
                    env.path("missing-profile").to_str().expect("missing path")
                ),
                "--output",
                output.to_str().expect("output path"),
            ])
            .output()
            .expect("failed to run ramune tail");

        // ASSERT
        assert!(!process_output.status.success());
        assert!(
            String::from_utf8_lossy(&process_output.stderr).contains("Failed to read input entry")
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
        assert!(process_output.status.success());
        let stdout = String::from_utf8_lossy(&process_output.stdout);
        assert!(stdout.contains("Usage: ramune <COMMAND>"));
        assert!(process_output.stderr.is_empty());
    }

    #[test]
    fn cli_version_exits_successfully() {
        // ACT
        let process_output = Command::new(ramune_bin())
            .arg("--version")
            .output()
            .expect("failed to run ramune --version");

        // ASSERT
        assert!(process_output.status.success());
        let stdout = String::from_utf8_lossy(&process_output.stdout);
        assert!(stdout.contains(env!("CARGO_PKG_VERSION")));
        assert!(process_output.stderr.is_empty());
    }

    #[test]
    fn run_from_create_writes_output() {
        // ARRANGE
        let env = TestEnv::new();
        let init = env.write("init", b"#!/bin/sh\nexec /sbin/init\n");
        let rootfs = env.write_rootfs();
        let output = env.path("run-from-initramfs.img");

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
        .expect("run_from create");

        // ASSERT
        assert!(output.exists());
    }

    #[test]
    fn run_from_tail_writes_output() {
        // ARRANGE
        let env = TestEnv::new();
        let profile = env.write("profile.toml", b"[overlay]\nname = \"test\"\n");
        let output = env.path("run-from-tail.img");

        // ACT
        cli::run_from([
            "ramune",
            "tail",
            "--entry",
            &format!("{}:profile.toml", profile.to_str().expect("profile path")),
            "--output",
            output.to_str().expect("output path"),
        ])
        .expect("run_from tail");

        // ASSERT
        assert!(output.exists());
    }

    #[test]
    fn run_from_tail_missing_source_errors() {
        // ARRANGE
        let env = TestEnv::new();
        let output = env.path("run-from-tail.img");

        // ACT
        let result = cli::run_from([
            "ramune",
            "tail",
            "--entry",
            &format!(
                "{}:profile.toml",
                env.path("missing-profile").to_str().expect("missing path")
            ),
            "--output",
            output.to_str().expect("output path"),
        ]);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn run_with_returns_zero_for_success() {
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
        ]);

        // ASSERT
        assert_eq!(exit_code, 0);
    }

    #[test]
    fn run_with_returns_one_for_error() {
        // ARRANGE
        let env = TestEnv::new();
        let output = env.path("run-with-tail.img");

        // ACT
        let exit_code = cli::run_with([
            "ramune",
            "tail",
            "--entry",
            &format!(
                "{}:profile.toml",
                env.path("missing-profile").to_str().expect("missing path")
            ),
            "--output",
            output.to_str().expect("output path"),
        ]);

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
