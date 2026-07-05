#[cfg(test)]
mod fixtures;

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::process::Command;

    use ramune::cli;

    use super::fixtures::{TestEnv, decode_initramfs};

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
        assert!(entries.iter().any(|entry| entry.0 == "init"));
        assert!(entries.iter().any(|entry| entry.0 == "rootfs.erofs"));
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
            String::from_utf8_lossy(&process_output.stderr).contains("Failed to open init binary")
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
        let output = env.path("run-with-error.img");

        // ACT
        let exit_code = cli::run_with([
            "ramune",
            "create",
            "--init",
            env.path("missing-init").to_str().expect("init path"),
            "--rootfs-dir",
            env.path("missing-rootfs").to_str().expect("rootfs path"),
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
