//! Extension discovery for squashfs images.
//!
//! Provides functions to discover and list extension squashfs images
//! from the extensions directory.

use std::path::Path;

/// Discover extension squashfs images in the default extensions directory.
pub fn discover_extensions() -> Vec<String> {
    discover_extensions_in(Path::new("/extensions"))
}

/// Discovers extension squashfs images in a directory.
pub fn discover_extensions_in(extensions_dir: &Path) -> Vec<String> {
    if !extensions_dir.exists() {
        return Vec::new();
    }

    let Ok(entries) = std::fs::read_dir(extensions_dir) else {
        return Vec::new();
    };

    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let is_sqsh = path.extension().and_then(|s| s.to_str()) == Some("sqsh");
            is_sqsh.then(|| path.to_str().map(String::from)).flatten()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn discover_extensions_finds_sqsh_files() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");

        std::fs::write(temp.path().join("app.sqsh"), b"").expect("Failed to create app.sqsh");
        std::fs::write(temp.path().join("lib.sqsh"), b"").expect("Failed to create lib.sqsh");
        std::fs::write(temp.path().join("tools.sqsh"), b"").expect("Failed to create tools.sqsh");
        std::fs::write(temp.path().join("readme.txt"), b"").expect("Failed to create readme.txt");
        std::fs::write(temp.path().join("config.json"), b"").expect("Failed to create config.json");

        // ACT
        let extensions = discover_extensions_in(temp.path());

        // ASSERT
        assert_eq!(extensions.len(), 3, "Should find 3 .sqsh files");
        assert!(
            extensions.iter().any(|e| e.ends_with("app.sqsh")),
            "Should find app.sqsh"
        );
        assert!(
            extensions.iter().any(|e| e.ends_with("lib.sqsh")),
            "Should find lib.sqsh"
        );
        assert!(
            extensions.iter().any(|e| e.ends_with("tools.sqsh")),
            "Should find tools.sqsh"
        );
    }

    #[test]
    fn discover_extensions_ignores_non_sqsh() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");

        std::fs::write(temp.path().join("file.tar.gz"), b"").unwrap();
        std::fs::write(temp.path().join("file.zip"), b"").unwrap();
        std::fs::write(temp.path().join("file.squashfs"), b"").unwrap();
        std::fs::write(temp.path().join("filesqsh"), b"").unwrap();

        // ACT
        let extensions = discover_extensions_in(temp.path());

        // ASSERT
        assert_eq!(extensions.len(), 0, "Should not find any .sqsh files");
    }

    #[test]
    fn discover_extensions_empty_directory() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");

        // ACT
        let extensions = discover_extensions_in(temp.path());

        // ASSERT
        assert_eq!(
            extensions.len(),
            0,
            "Should return empty vector for empty directory"
        );
    }

    #[test]
    fn discover_extensions_nonexistent_directory() {
        // ARRANGE
        let nonexistent = Path::new("/nonexistent/extensions");

        // ACT
        let extensions = discover_extensions_in(nonexistent);

        // ASSERT
        assert_eq!(
            extensions.len(),
            0,
            "Should return empty vector for nonexistent directory"
        );
    }

    #[test]
    fn discover_extensions_nested_directories() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");

        std::fs::create_dir(temp.path().join("subdir")).unwrap();
        std::fs::write(temp.path().join("root.sqsh"), b"").unwrap();
        std::fs::write(temp.path().join("subdir/nested.sqsh"), b"").unwrap();

        // ACT
        let extensions = discover_extensions_in(temp.path());

        // ASSERT
        assert_eq!(
            extensions.len(),
            1,
            "Should only find .sqsh files in root, not subdirectories"
        );
        assert!(
            extensions[0].ends_with("root.sqsh"),
            "Should find root.sqsh"
        );
    }

    #[test]
    fn discover_extensions_returns_full_paths() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");

        std::fs::write(temp.path().join("test.sqsh"), b"").unwrap();

        // ACT
        let extensions = discover_extensions_in(temp.path());

        // ASSERT
        assert_eq!(extensions.len(), 1);
        assert!(
            extensions[0].starts_with(temp.path().to_str().unwrap()),
            "Should return full path, not just filename"
        );
        assert!(
            extensions[0].ends_with("test.sqsh"),
            "Path should end with test.sqsh"
        );
    }

    #[test]
    fn discover_extensions_case_sensitive() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");

        std::fs::write(temp.path().join("lowercase.sqsh"), b"").unwrap();
        std::fs::write(temp.path().join("uppercase.SQSH"), b"").unwrap();
        std::fs::write(temp.path().join("mixed.Sqsh"), b"").unwrap();

        // ACT
        let extensions = discover_extensions_in(temp.path());

        // ASSERT
        assert_eq!(
            extensions.len(),
            1,
            "Should only match lowercase .sqsh extension"
        );
        assert!(extensions[0].ends_with("lowercase.sqsh"));
    }
}
