#[cfg(test)]
mod fixtures;

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Cursor;
    use std::path::Path;

    use ramune::error::RamuneError;

    use super::fixtures::{TestEnv, decode_initramfs, parse_newc_archive};

    fn create_config<'a>(
        init: &'a [u8],
        rootfs_dir: &'a Path,
        file_contexts: Option<&'a erofs::FileContexts>,
    ) -> ramune::CreateConfig<'a> {
        ramune::CreateConfig {
            init,
            rootfs_dir,
            file_contexts,
            compression_level: 19,
            rootfs_compression_level: erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL,
        }
    }

    fn append_entry<'a>(
        archive_path: &'a Path,
        reader: &'a mut Cursor<Vec<u8>>,
    ) -> ramune::AppendEntry<'a> {
        ramune::AppendEntry {
            archive_path,
            mode: 0o100_644,
            len: u64::try_from(reader.get_ref().len()).unwrap_or(0),
            reader,
        }
    }

    #[test]
    fn create_writes_expected_archive_entries() {
        // ARRANGE
        let env = TestEnv::new();
        let init_path = env.write("init", b"#!/bin/sh\nexec /sbin/init\n");
        let init_bytes = std::fs::read(&init_path).expect("read init");
        let rootfs = env.write_rootfs();
        let output = env.path("initramfs.img");

        // ACT
        let mut buf = Vec::new();
        ramune::create(&create_config(&init_bytes, &rootfs, None), &mut buf)
            .expect("create should succeed");
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

        // ACT
        let mut buf = Vec::new();
        ramune::create(
            &create_config(&init_bytes, &rootfs, Some(&contexts)),
            &mut buf,
        )
        .expect("create should succeed with file contexts");
        std::fs::write(&output, &buf).expect("write output");

        // ASSERT
        let entries = decode_initramfs(&output);
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn create_returns_error_for_missing_rootfs() {
        // ARRANGE
        let env = TestEnv::new();
        let init_bytes = b"#!/bin/sh\nexec /sbin/init\n";
        let missing_rootfs = env.path("nonexistent-rootfs");

        // ACT
        let mut buf = Vec::new();
        let result = ramune::create(
            &ramune::CreateConfig {
                init: init_bytes.as_slice(),
                rootfs_dir: &missing_rootfs,
                file_contexts: None,
                compression_level: 19,
                rootfs_compression_level: 3,
            },
            &mut buf,
        );

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn build_tail_returns_empty_when_no_entries() {
        // ARRANGE
        let mut entries: [ramune::AppendEntry<'_>; 0] = [];

        // ACT
        let tail = ramune::build_tail(&mut ramune::TailConfig {
            entries: &mut entries,
            compression_level: 19,
        })
        .expect("build tail should succeed without entries");

        // ASSERT
        assert!(tail.is_empty());
    }

    #[test]
    fn build_tail_with_entries_writes_named_archive() {
        // ARRANGE
        let env = TestEnv::new();
        let profile_data =
            fs::read(env.write("profile.toml", b"profile = true\n")).expect("read profile");
        let extension_data =
            fs::read(env.write("test-ext.erofs", b"erofs-bytes")).expect("read extension");
        let mut profile_reader = Cursor::new(profile_data.clone());
        let mut extension_reader = Cursor::new(extension_data.clone());
        let mut entries = [
            append_entry(Path::new("profile.toml"), &mut profile_reader),
            append_entry(
                Path::new("extensions/test-ext.erofs"),
                &mut extension_reader,
            ),
        ];

        // ACT
        let tail = ramune::build_tail(&mut ramune::TailConfig {
            entries: &mut entries,
            compression_level: 19,
        })
        .expect("build tail should succeed");

        // ASSERT
        let archive = zstd::decode_all(tail.as_slice()).expect("decode tail");
        let parsed = parse_newc_archive(&archive);
        let names: Vec<&str> = parsed.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["extensions/test-ext.erofs", "profile.toml"]);
        assert_eq!(parsed.first().expect("first entry").mode, 0o100_644);
        assert_eq!(parsed.first().expect("first entry").data, extension_data);
        assert_eq!(parsed.get(1).expect("second entry").data, profile_data);
    }

    #[test]
    fn build_tail_returns_error_for_short_reader() {
        // ARRANGE
        let mut reader = Cursor::new(b"small".to_vec());
        let mut entries = [ramune::AppendEntry {
            archive_path: Path::new("profile.toml"),
            mode: 0o100_644,
            len: 64,
            reader: &mut reader,
        }];

        // ACT
        let result = ramune::build_tail(&mut ramune::TailConfig {
            entries: &mut entries,
            compression_level: 19,
        });

        // ASSERT
        assert!(
            matches!(result, Err(RamuneError::CpioError(message)) if message.contains("ended early"))
        );
    }
}
