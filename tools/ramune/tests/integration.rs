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
        let init_len = init_bytes.len().try_into().unwrap_or(u64::MAX);
        let erofs_len = rootfs_erofs.len().try_into().unwrap_or(u64::MAX);
        let mut init_reader = Cursor::new(init_bytes);
        let mut erofs_reader = Cursor::new(rootfs_erofs);
        let mut entries = [
            ramune::EntryStream::new(Path::new("init"), 0o100_755, &mut init_reader, init_len),
            ramune::EntryStream::new(
                Path::new("rootfs.erofs"),
                0o100_644,
                &mut erofs_reader,
                erofs_len,
            ),
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
        let init_len = init_bytes.len().try_into().unwrap_or(u64::MAX);
        let erofs_len = rootfs_erofs.len().try_into().unwrap_or(u64::MAX);
        let mut init_reader = Cursor::new(init_bytes);
        let mut erofs_reader = Cursor::new(rootfs_erofs);
        let mut entries = [
            ramune::EntryStream::new(Path::new("init"), 0o100_755, &mut init_reader, init_len),
            ramune::EntryStream::new(
                Path::new("rootfs.erofs"),
                0o100_644,
                &mut erofs_reader,
                erofs_len,
            ),
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
        let profile_len = profile_data.len().try_into().unwrap_or(u64::MAX);
        let extension_len = extension_data.len().try_into().unwrap_or(u64::MAX);
        let mut profile_reader = Cursor::new(profile_data.clone());
        let mut extension_reader = Cursor::new(extension_data.clone());
        let mut entries = [
            ramune::EntryStream::new(
                Path::new("profile.toml"),
                0o100_644,
                &mut profile_reader,
                profile_len,
            ),
            ramune::EntryStream::new(
                Path::new("extensions/test-ext.erofs"),
                0o100_644,
                &mut extension_reader,
                extension_len,
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
        let mut entries = [ramune::EntryStream::new(
            Path::new("profile.toml"),
            0o100_644,
            &mut reader,
            64,
        )];

        // ACT
        let mut buf = Vec::new();
        let result = ramune::archive(&mut entries, 19, &mut buf);

        // ASSERT
        assert!(
            matches!(result, Err(RamuneError::CpioError(message)) if message.contains("ended early"))
        );
    }
}
