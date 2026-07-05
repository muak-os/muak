#[cfg(test)]
mod fixtures;

#[cfg(test)]
mod tests {
    use std::io::Read;

    use erofs::writer;
    use ramune::archive;
    use ramune::error::RamuneError;
    use ramune::rootfs;

    use super::fixtures::{TestEnv, decode_initramfs, parse_newc_archive};

    #[test]
    fn create_writes_expected_archive_entries() {
        // ARRANGE
        let env = TestEnv::new();
        let init_path = env.write("init", b"#!/bin/sh\nexec /sbin/init\n");
        let mut init_file = std::fs::File::open(&init_path).expect("open init");
        let init_len = init_file.metadata().expect("init metadata").len();
        let rootfs = env.write_rootfs();
        let output = env.path("initramfs.img");

        let (erofs_plan, erofs_config, erofs_len) =
            rootfs::prepare_and_plan(&rootfs, None, erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL)
                .expect("prepare rootfs");

        let mut entries = [
            ramune::Entry {
                path: "init".into(),
                mode: 0o100_755,
                len: init_len,
            },
            ramune::Entry {
                path: "rootfs.erofs".into(),
                mode: 0o100_644,
                len: erofs_len,
            },
        ];

        // ACT
        let mut buf = Vec::new();
        archive::compressed(&mut entries, &mut buf, 19, |entry, w| {
            match entry.path.as_str() {
                "init" => std::io::copy(&mut (&mut init_file).take(init_len), w)
                    .map(|_| ())
                    .map_err(|e| RamuneError::WriteError {
                        file: String::new(),
                        source: e,
                    }),
                "rootfs.erofs" => writer::image(w, &erofs_plan, &erofs_config)
                    .map_err(|e| RamuneError::ErofsError(e.to_string())),
                other => panic!("unexpected entry: {other}"),
            }
        })
        .expect("archive should succeed");
        std::fs::write(&output, &buf).expect("write output");

        // ASSERT
        let entries = decode_initramfs(&output);
        let names: Vec<&str> = entries.iter().map(|entry| entry.0.as_str()).collect();
        assert_eq!(names, ["init", "rootfs.erofs"]);

        let init_entry = entries
            .iter()
            .find(|entry| entry.0 == "init")
            .expect("missing init entry");
        assert_eq!(init_entry.1, 0o100_755);
        assert_eq!(init_entry.2, b"#!/bin/sh\nexec /sbin/init\n");
    }

    #[test]
    fn create_supports_file_contexts() {
        // ARRANGE
        let env = TestEnv::new();
        let init_path = env.write("init", b"#!/bin/sh\nexec /sbin/init\n");
        let mut init_file = std::fs::File::open(&init_path).expect("open init");
        let init_len = init_file.metadata().expect("init metadata").len();
        let rootfs = env.write_rootfs();
        let output = env.path("initramfs.img");
        let contexts =
            erofs::FileContexts::from_reader(b"/.*    system_u:object_r:file_t:s0\n".as_slice())
                .expect("file contexts should parse");

        let (erofs_plan, erofs_config, erofs_len) =
            rootfs::prepare_and_plan(&rootfs, Some(&contexts), 3).expect("prepare rootfs");

        let mut entries = [
            ramune::Entry {
                path: "init".into(),
                mode: 0o100_755,
                len: init_len,
            },
            ramune::Entry {
                path: "rootfs.erofs".into(),
                mode: 0o100_644,
                len: erofs_len,
            },
        ];

        // ACT
        let mut buf = Vec::new();
        archive::compressed(&mut entries, &mut buf, 19, |entry, w| {
            match entry.path.as_str() {
                "init" => std::io::copy(&mut (&mut init_file).take(init_len), w)
                    .map(|_| ())
                    .map_err(|e| RamuneError::WriteError {
                        file: String::new(),
                        source: e,
                    }),
                "rootfs.erofs" => writer::image(w, &erofs_plan, &erofs_config)
                    .map_err(|e| RamuneError::ErofsError(e.to_string())),
                other => panic!("unexpected entry: {other}"),
            }
        })
        .expect("archive should succeed");
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
        rootfs::prepare_and_plan(&missing_rootfs, None, 3).unwrap_err();
    }

    #[test]
    fn archive_empty_entries() {
        // ARRANGE / ACT
        let mut buf = Vec::new();
        archive::cpio(&mut [], &mut buf, |_, _| Ok(())).unwrap();

        // ASSERT
        assert!(buf.is_empty());
    }

    #[test]
    fn archive_with_entries_writes_named_archive() {
        // ARRANGE
        let env = TestEnv::new();
        let profile_data = b"profile = true\n".to_vec();
        let extension_data = b"erofs-bytes".to_vec();
        let profile_path = env.write("profile.toml", &profile_data);
        let extension_path = env.write("test-ext.erofs", &extension_data);
        let mut profile_file = std::fs::File::open(&profile_path).expect("open profile");
        let mut extension_file = std::fs::File::open(&extension_path).expect("open extension");
        let profile_len = profile_file.metadata().expect("profile metadata").len();
        let extension_len = extension_file.metadata().expect("extension metadata").len();

        let mut entries = [
            ramune::Entry {
                path: "profile.toml".into(),
                mode: 0o100_644,
                len: profile_len,
            },
            ramune::Entry {
                path: "extensions/test-ext.erofs".into(),
                mode: 0o100_644,
                len: extension_len,
            },
        ];

        // ACT
        let mut buf = Vec::new();
        archive::cpio(&mut entries, &mut buf, |entry, w| {
            let reader: &mut dyn Read = match entry.path.as_str() {
                "profile.toml" => &mut profile_file,
                "extensions/test-ext.erofs" => &mut extension_file,
                other => panic!("unexpected entry: {other}"),
            };
            let mut limited = reader.take(entry.len);
            std::io::copy(&mut limited, w).map_err(|e| RamuneError::WriteError {
                file: String::new(),
                source: e,
            })?;
            Ok(())
        })
        .expect("write_cpio should succeed");

        // ASSERT
        let parsed = parse_newc_archive(&buf);
        let names: Vec<&str> = parsed.iter().map(|entry| entry.0.as_str()).collect();
        assert_eq!(names, ["extensions/test-ext.erofs", "profile.toml"]);
        assert_eq!(parsed.first().expect("first entry").1, 0o100_644);
        assert_eq!(parsed.first().expect("first entry").2, extension_data);
        assert_eq!(parsed.get(1).expect("second entry").2, profile_data);
    }
}
