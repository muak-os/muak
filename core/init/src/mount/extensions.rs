//! Extension discovery for EROFS images.

use std::path::Path;

/// Discover extension EROFS images in the default extensions directory.
pub fn discover_extensions() -> Vec<String> {
    discover_extensions_in(Path::new("/extensions"))
}

/// Discovers extension EROFS images in a directory.
pub fn discover_extensions_in(extensions_dir: &Path) -> Vec<String> {
    if !extensions_dir.exists() {
        return Vec::new();
    }

    let Ok(entries) = std::fs::read_dir(extensions_dir) else {
        return Vec::new();
    };

    let mut result: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let is_erofs = path.extension().and_then(|s| s.to_str()) == Some("erofs");
            is_erofs.then(|| path.to_str().map(String::from)).flatten()
        })
        .collect();

    result.sort_unstable();
    result
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn discover_extensions_finds_erofs_files() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");

        std::fs::write(temp.path().join("app.erofs"), b"").expect("Failed to create app.erofs");
        std::fs::write(temp.path().join("lib.erofs"), b"").expect("Failed to create lib.erofs");
        std::fs::write(temp.path().join("tools.erofs"), b"").expect("Failed to create tools.erofs");
        std::fs::write(temp.path().join("readme.txt"), b"").expect("Failed to create readme.txt");
        std::fs::write(temp.path().join("config.json"), b"").expect("Failed to create config.json");

        // ACT
        let extensions = discover_extensions_in(temp.path());

        // ASSERT
        assert_eq!(extensions.len(), 3, "Should find 3 .erofs files");
        assert!(extensions.iter().any(|e| e.ends_with("app.erofs")));
        assert!(extensions.iter().any(|e| e.ends_with("lib.erofs")));
        assert!(extensions.iter().any(|e| e.ends_with("tools.erofs")));
        let names: Vec<&str> = extensions
            .iter()
            .map(|p| p.rsplit('/').next().unwrap())
            .collect();
        assert_eq!(
            names,
            ["app.erofs", "lib.erofs", "tools.erofs"],
            "Results must be sorted"
        );
    }

    #[test]
    fn discover_extensions_ignores_non_erofs() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");

        std::fs::write(temp.path().join("file.tar.gz"), b"").unwrap();
        std::fs::write(temp.path().join("file.zip"), b"").unwrap();
        std::fs::write(temp.path().join("file.squashfs"), b"").unwrap();
        std::fs::write(temp.path().join("file.sqsh"), b"").unwrap();
        std::fs::write(temp.path().join("fileerofs"), b"").unwrap();

        // ACT
        let extensions = discover_extensions_in(temp.path());

        // ASSERT
        assert_eq!(extensions.len(), 0, "Should not find any .erofs files");
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
        std::fs::write(temp.path().join("root.erofs"), b"").unwrap();
        std::fs::write(temp.path().join("subdir/nested.erofs"), b"").unwrap();

        // ACT
        let extensions = discover_extensions_in(temp.path());

        // ASSERT
        assert_eq!(
            extensions.len(),
            1,
            "Should only find .erofs files in root, not subdirectories"
        );
        assert!(
            extensions[0].ends_with("root.erofs"),
            "Should find root.erofs"
        );
    }

    #[test]
    fn discover_extensions_returns_full_paths() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");

        std::fs::write(temp.path().join("test.erofs"), b"").unwrap();

        // ACT
        let extensions = discover_extensions_in(temp.path());

        // ASSERT
        assert_eq!(extensions.len(), 1);
        assert!(
            extensions[0].starts_with(temp.path().to_str().unwrap()),
            "Should return full path, not just filename"
        );
        assert!(
            extensions[0].ends_with("test.erofs"),
            "Path should end with test.erofs"
        );
    }

    #[test]
    fn discover_extensions_case_sensitive() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");

        std::fs::write(temp.path().join("lowercase.erofs"), b"").unwrap();
        std::fs::write(temp.path().join("uppercase.EROFS"), b"").unwrap();
        std::fs::write(temp.path().join("mixed.Erofs"), b"").unwrap();

        // ACT
        let extensions = discover_extensions_in(temp.path());

        // ASSERT
        assert_eq!(
            extensions.len(),
            1,
            "Should only match lowercase .erofs extension"
        );
        assert!(extensions[0].ends_with("lowercase.erofs"));
    }
}
