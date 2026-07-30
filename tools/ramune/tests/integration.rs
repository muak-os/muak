#[cfg(test)]
mod fixtures;

#[cfg(test)]
mod tests {
    use std::io::Read;

    use ramune::archive;
    use ramune::error::RamuneError;

    use super::fixtures::{TestEnv, parse_newc_archive};

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
