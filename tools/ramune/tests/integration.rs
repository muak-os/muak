#[cfg(test)]
mod fixtures;

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use ramune::error::RamuneError;

    use super::fixtures::{TestEnv, decode_extension_archive, decode_initramfs};

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

    fn extra_file<'a>(name: &str, path: &'a Path, compress: bool) -> ramune::ExtraFile<'a> {
        ramune::ExtraFile {
            name: name.to_owned(),
            path,
            compress,
        }
    }

    fn extend_config<'a>(
        base: &'a Path,
        extra_files: &'a [ramune::ExtraFile<'a>],
    ) -> ramune::ExtendConfig<'a> {
        ramune::ExtendConfig {
            base,
            extra_files,
            compression_level: 19,
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

        let rootfs_entry = entries
            .iter()
            .find(|entry| entry.name == "rootfs.erofs")
            .expect("missing rootfs erofs entry");
        assert_eq!(rootfs_entry.mode, 0o100_644);
        assert!(
            !rootfs_entry.data.is_empty(),
            "rootfs erofs should not be empty"
        );
        assert_eq!(
            rootfs_entry.data.len().rem_euclid(4096),
            0,
            "rootfs erofs should be block aligned"
        );
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
    fn extend_without_extra_files_streams_base_to_output() {
        // ARRANGE
        let env = TestEnv::new();
        let base = env.write("base.img", b"base-initramfs");
        let output = env.path("copy.img");
        let config = extend_config(base.as_path(), &[]);

        // ACT
        let mut file = std::fs::File::create(&output).expect("create output");
        ramune::extend(&config, &mut file).expect("extend should succeed without extra files");

        // ASSERT
        let result = fs::read(&output).expect("read output");
        assert_eq!(result, b"base-initramfs");
    }

    #[test]
    fn extend_with_compress_dir_appends_named_archive() {
        // ARRANGE
        let env = TestEnv::new();
        let base_bytes = b"base-initramfs";
        let base = env.write("base.img", base_bytes);
        let output = env.path("extended.img");
        let extension = env.write_extension("test-ext", b"hello extension");

        let extras = [extra_file(
            "extensions/test-ext.erofs",
            extension.as_path(),
            true,
        )];
        let config = extend_config(base.as_path(), &extras);

        // ACT
        let mut file = std::fs::File::create(&output).expect("create output");
        ramune::extend(&config, &mut file).expect("extend should succeed with extensions");

        // ASSERT
        let image = fs::read(&output).expect("read extended image");
        assert!(image.starts_with(base_bytes));

        let entries = decode_extension_archive(&output, base_bytes.len());
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["extensions/test-ext.erofs"]);

        let extension_entry = entries
            .iter()
            .find(|entry| entry.name == "extensions/test-ext.erofs")
            .expect("missing extension entry");
        assert_eq!(extension_entry.mode, 0o100_644);
        assert!(!extension_entry.data.is_empty());
        assert_eq!(extension_entry.data.len().rem_euclid(4096), 0);
    }

    #[test]
    fn extend_in_place_appends_archive() {
        // ARRANGE
        let env = TestEnv::new();
        let base_bytes = b"base-initramfs";
        let image = env.write("base.img", base_bytes);
        let tmp_output = env.path("extended.img");
        let extension = env.write_extension("in-place-ext", b"payload");

        let extras = [extra_file(
            "extensions/in-place-ext.erofs",
            extension.as_path(),
            true,
        )];
        let config = extend_config(image.as_path(), &extras);

        // ACT
        let mut file = std::fs::File::create(&tmp_output).expect("create output");
        ramune::extend(&config, &mut file).expect("extend should succeed");

        // ASSERT
        let output = fs::read(&tmp_output).expect("read output");
        assert!(output.starts_with(base_bytes));

        let entries = decode_extension_archive(&tmp_output, base_bytes.len());
        assert!(
            entries
                .iter()
                .any(|entry| entry.name == "extensions/in-place-ext.erofs")
        );
    }

    #[test]
    fn extend_returns_read_error_for_missing_base() {
        // ARRANGE
        let env = TestEnv::new();
        let output = env.path("extended.img");
        let config = extend_config(Path::new("/nonexistent/base.img"), &[]);

        // ACT
        let mut file = std::fs::File::create(&output).expect("create output");
        let result = ramune::extend(&config, &mut file);

        // ASSERT
        assert!(matches!(result, Err(RamuneError::ReadError { .. })));
    }
}
