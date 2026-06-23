#[cfg(test)]
mod fixtures;

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Cursor;
    use std::path::Path;

    use ramune::error::RamuneError;
    use ramune::rootfs;

    use super::fixtures::{TestEnv, decode_initramfs, parse_newc_archive};

    #[test]
    fn create_writes_expected_archive_entries() {
        // ARRANGE
        let env = TestEnv::new();
        let init_path = env.write("init", b"#!/bin/sh\nexec /sbin/init\n");
        let init_bytes = std::fs::read(&init_path).expect("read init");
        let rootfs = env.write_rootfs();
        let output = env.path("initramfs.img");

        let rootfs_erofs = rootfs::prepare(&rootfs, None, erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL)
            .expect("prepare rootfs");
        let mut init_reader = Cursor::new(init_bytes);
        let mut erofs_reader = Cursor::new(rootfs_erofs);
        let mut entries = [
            ramune::Entry::from_bytes(Path::new("init"), 0o100_755, &mut init_reader),
            ramune::Entry::from_bytes(Path::new("rootfs.erofs"), 0o100_644, &mut erofs_reader),
        ];

        // ACT
        let mut buf = Vec::new();
        ramune::archive(&mut entries, 19, &mut buf).expect("archive should succeed");
        std::fs::write(&output, &buf).expect("write output");

        // ASSERT
        let entries = decode_initramfs(&output);
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["init", "rootfs.erofs"]);

        let init_entry = entries
            .iter()
            .find(|entry| entry.name == "init")
            .expect("missing init entry");
        assert_eq!(init_entry.mode, 0o100_755);
        assert_eq!(init_entry.data, b"#!/bin/sh\nexec /sbin/init\n");
    }

    #[test]
    fn create_supports_file_contexts() {
        // ARRANGE
        let env = TestEnv::new();
        let init_path = env.write("init", b"#!/bin/sh\nexec /sbin/init\n");
        let init_bytes = std::fs::read(&init_path).expect("read init");
        let rootfs = env.write_rootfs();
        let output = env.path("initramfs.img");
        let contexts =
            erofs::FileContexts::from_reader(b"/.*    system_u:object_r:file_t:s0\n".as_slice())
                .expect("file contexts should parse");

        let rootfs_erofs = rootfs::prepare(&rootfs, Some(&contexts), 3).expect("prepare rootfs");
        let mut init_reader = Cursor::new(init_bytes);
        let mut erofs_reader = Cursor::new(rootfs_erofs);
        let mut entries = [
            ramune::Entry::from_bytes(Path::new("init"), 0o100_755, &mut init_reader),
            ramune::Entry::from_bytes(Path::new("rootfs.erofs"), 0o100_644, &mut erofs_reader),
        ];

        // ACT
        let mut buf = Vec::new();
        ramune::archive(&mut entries, 19, &mut buf).expect("archive should succeed");
        std::fs::write(&output, &buf).expect("write output");

        // ASSERT
        let entries = decode_initramfs(&output);
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn create_returns_error_for_missing_rootfs() {
        // ARRANGE
        let env = TestEnv::new();
        let missing_rootfs = env.path("nonexistent-rootfs");

        // ACT / ASSERT
        rootfs::prepare(&missing_rootfs, None, 3).unwrap_err();
    }

    #[test]
    fn archive_empty_entries() {
        // ARRANGE / ACT
        let mut buf = Vec::new();
        ramune::archive(&mut [], 19, &mut buf).unwrap();

        // ASSERT
        assert!(buf.is_empty());
    }

    #[test]
    fn archive_with_entries_writes_named_archive() {
        // ARRANGE
        let env = TestEnv::new();
        let profile_data =
            fs::read(env.write("profile.toml", b"profile = true\n")).expect("read profile");
        let extension_data =
            fs::read(env.write("test-ext.erofs", b"erofs-bytes")).expect("read extension");
        let mut profile_reader = Cursor::new(profile_data.clone());
        let mut extension_reader = Cursor::new(extension_data.clone());
        let mut entries = [
            ramune::Entry::from_bytes(Path::new("profile.toml"), 0o100_644, &mut profile_reader),
            ramune::Entry::from_bytes(
                Path::new("extensions/test-ext.erofs"),
                0o100_644,
                &mut extension_reader,
            ),
        ];

        // ACT
        let mut buf = Vec::new();
        ramune::archive(&mut entries, 19, &mut buf).expect("archive should succeed");

        // ASSERT
        let archive = zstd::decode_all(buf.as_slice()).expect("decode tail");
        let parsed = parse_newc_archive(&archive);
        let names: Vec<&str> = parsed.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["extensions/test-ext.erofs", "profile.toml"]);
        assert_eq!(parsed.first().expect("first entry").mode, 0o100_644);
        assert_eq!(parsed.first().expect("first entry").data, extension_data);
        assert_eq!(parsed.get(1).expect("second entry").data, profile_data);
    }

    #[test]
    fn archive_returns_error_for_short_reader() {
        // ARRANGE
        let mut reader = Cursor::new(b"small".to_vec());
        let mut entries = [ramune::Entry {
            archive_path: Path::new("profile.toml"),
            mode: 0o100_644,
            len: 64,
            reader: &mut reader,
        }];

        // ACT
        let mut buf = Vec::new();
        let result = ramune::archive(&mut entries, 19, &mut buf);

        // ASSERT
        assert!(
            matches!(result, Err(RamuneError::CpioError(message)) if message.contains("ended early"))
        );
    }
}
