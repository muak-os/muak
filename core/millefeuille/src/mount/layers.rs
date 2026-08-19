//! Discover image files to stack as additional overlay layers.

use std::collections::HashSet;
use std::path::Path;

use super::IMAGE_EXTENSION;

/// Discovers every image file in the ramfs root excluding the base rootfs image and live mountpoints.
pub fn discover_layers() -> Vec<String> {
    discover_layers_in(Path::new("/"), &default_skip_set())
}

/// Recursively discovers image files under `root`, skipping any path present in `skip`.
fn discover_layers_in(root: &Path, skip: &HashSet<String>) -> Vec<String> {
    let mut images = Vec::new();
    walk(root, "", skip, &mut images);
    images.sort_unstable();

    images
}

fn walk(dir: &Path, rel: &str, skip: &HashSet<String>, images: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel_path = if rel.is_empty() {
            name
        } else {
            format!("{rel}/{name}")
        };

        if skip.contains(&rel_path) {
            continue;
        }

        let Ok(file_type) = entry.file_type() else {
            continue;
        };

        if file_type.is_dir() {
            walk(&path, &rel_path, skip, images);
            continue;
        }

        if file_type.is_symlink()
            || path.extension().and_then(|ext| ext.to_str()) != Some(IMAGE_EXTENSION)
        {
            continue;
        }

        images.push(path.to_string_lossy().into_owned());
    }
}

fn default_skip_set() -> HashSet<String> {
    let mut skip = HashSet::new();

    if let Ok(mounts) = std::fs::read_to_string("/proc/self/mounts") {
        for relative in mounts
            .lines()
            .filter_map(|line| line.split_whitespace().nth(1))
            .map(|mountpoint| mountpoint.trim_start_matches('/'))
            .filter(|relative| !relative.is_empty())
        {
            skip.insert(relative.to_owned());
        }
    }

    skip.insert(format!("rootfs.{IMAGE_EXTENSION}"));

    skip
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::os::unix::fs::symlink;

    use tempfile::TempDir;

    use super::*;

    fn skip(entries: &[&str]) -> HashSet<String> {
        entries.iter().map(|entry| (*entry).to_owned()).collect()
    }

    #[test]
    fn discovers_layers_in_nested_directories() {
        // ARRANGE
        let temp = TempDir::new().expect("create tempdir");
        let top = format!("app.{IMAGE_EXTENSION}");
        let nested = format!("lib/tools.{IMAGE_EXTENSION}");
        std::fs::write(temp.path().join(&top), b"").expect("create top image");
        std::fs::create_dir_all(temp.path().join("lib")).unwrap();
        std::fs::write(temp.path().join(&nested), b"").expect("create nested image");
        std::fs::write(temp.path().join("readme.txt"), b"").unwrap();

        // ACT
        let layers = discover_layers_in(temp.path(), &skip(&[]));

        // ASSERT
        assert_eq!(layers.len(), 2, "Should find 2 image files");
        assert!(layers.iter().any(|layer| layer.ends_with(&top)));
        assert!(layers.iter().any(|layer| layer.ends_with(&nested)));
    }

    #[test]
    fn skips_directories_listed_in_skip_set() {
        // ARRANGE
        let temp = TempDir::new().expect("create tempdir");
        std::fs::create_dir_all(temp.path().join("mounted")).unwrap();
        std::fs::create_dir_all(temp.path().join("free")).unwrap();
        let mounted = format!("mounted/app.{IMAGE_EXTENSION}");
        let free = format!("free/app.{IMAGE_EXTENSION}");
        std::fs::write(temp.path().join(&mounted), b"").expect("create mounted image");
        std::fs::write(temp.path().join(&free), b"").expect("create free image");

        // ACT
        let layers = discover_layers_in(temp.path(), &skip(&["mounted"]));

        // ASSERT
        assert_eq!(layers.len(), 1, "Should skip the mounted directory");
        assert!(layers.iter().all(|layer| layer.ends_with(&free)));
    }

    #[test]
    fn skips_nested_directories_listed_in_skip_set() {
        // ARRANGE
        let temp = TempDir::new().expect("create tempdir");
        std::fs::create_dir_all(temp.path().join("srv/deep")).unwrap();
        let shallow = format!("srv/ext.{IMAGE_EXTENSION}");
        let deep = format!("srv/deep/x.{IMAGE_EXTENSION}");
        std::fs::write(temp.path().join(&shallow), b"").expect("create shallow image");
        std::fs::write(temp.path().join(&deep), b"").expect("create deep image");

        // ACT
        let layers = discover_layers_in(temp.path(), &skip(&["srv/deep"]));

        // ASSERT
        assert_eq!(layers.len(), 1, "Should skip the nested mountpoint path");
        assert!(layers.iter().all(|layer| layer.ends_with(&shallow)));
    }

    #[test]
    fn skips_files_listed_in_skip_set() {
        // ARRANGE
        let temp = TempDir::new().expect("create tempdir");
        let base = format!("rootfs.{IMAGE_EXTENSION}");
        let extra = format!("app.{IMAGE_EXTENSION}");
        std::fs::write(temp.path().join(&base), b"").expect("create base image");
        std::fs::write(temp.path().join(&extra), b"").expect("create extra image");
        let mut skip_set = skip(&[]);
        skip_set.insert(base.clone());

        // ACT
        let layers = discover_layers_in(temp.path(), &skip_set);

        // ASSERT
        assert_eq!(layers.len(), 1, "Should skip the base image file");
        assert!(!layers.iter().any(|layer| layer.ends_with(&base)));
        assert!(layers.iter().all(|layer| layer.ends_with(&extra)));
    }

    #[test]
    fn ignores_non_image_files() {
        // ARRANGE
        let temp = TempDir::new().expect("create tempdir");
        let uppercase = format!("upper.{}", IMAGE_EXTENSION.to_uppercase());
        std::fs::write(temp.path().join("file.tar.gz"), b"").unwrap();
        std::fs::write(temp.path().join("fileerofs"), b"").unwrap();
        std::fs::write(temp.path().join(uppercase), b"").unwrap();

        // ACT
        let layers = discover_layers_in(temp.path(), &skip(&[]));

        // ASSERT
        assert_eq!(
            layers.len(),
            0,
            "Should only match lowercase image extension"
        );
    }

    #[test]
    fn sorts_results_by_path() {
        // ARRANGE
        let temp = TempDir::new().expect("create tempdir");
        std::fs::create_dir_all(temp.path().join("a")).unwrap();
        let top = format!("z.{IMAGE_EXTENSION}");
        let deep = format!("a/b.{IMAGE_EXTENSION}");
        let shallow = format!("a.{IMAGE_EXTENSION}");
        std::fs::write(temp.path().join(&top), b"").expect("create top image");
        std::fs::write(temp.path().join(&deep), b"").expect("create deep image");
        std::fs::write(temp.path().join(&shallow), b"").expect("create shallow image");

        // ACT
        let layers = discover_layers_in(temp.path(), &skip(&[]));

        // ASSERT
        let expected = [shallow, deep, top]
            .into_iter()
            .map(|name| temp.path().join(name).to_string_lossy().into_owned())
            .collect::<Vec<String>>();
        assert_eq!(layers, expected, "Layers must be sorted by full path");
    }

    #[test]
    fn empty_root_returns_empty() {
        // ARRANGE
        let temp = TempDir::new().expect("create tempdir");

        // ACT
        let layers = discover_layers_in(temp.path(), &skip(&[]));

        // ASSERT
        assert_eq!(layers.len(), 0, "Should return empty for empty directory");
    }

    #[test]
    fn nonexistent_root_returns_empty() {
        // ARRANGE
        let nonexistent = Path::new("/nonexistent/layers");

        // ACT
        let layers = discover_layers_in(nonexistent, &skip(&[]));

        // ASSERT
        assert_eq!(
            layers.len(),
            0,
            "Should return empty for nonexistent directory"
        );
    }

    #[test]
    fn does_not_follow_symlinks() {
        // ARRANGE
        let temp = TempDir::new().expect("create tempdir");
        let hidden = TempDir::new().expect("create hidden tempdir");
        let real = format!("real.{IMAGE_EXTENSION}");
        let hidden_file = format!("x.{IMAGE_EXTENSION}");
        std::fs::write(temp.path().join(&real), b"").expect("create real image");
        std::fs::write(hidden.path().join(&hidden_file), b"").expect("create hidden image");
        symlink(&real, temp.path().join("link.erofs")).expect("create file symlink");
        symlink(hidden.path(), temp.path().join("dirlink")).expect("create dir symlink");

        // ACT
        let layers = discover_layers_in(temp.path(), &skip(&[]));

        // ASSERT
        assert_eq!(layers.len(), 1, "Symlinks must not be followed");
        assert!(layers.iter().all(|layer| layer.ends_with(&real)));
    }
}
