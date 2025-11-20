use std::error::Error;
use std::path::Path;
use walkdir::WalkDir;

use crate::manifest::ExtensionManifest;

pub fn overlay_extension(
    ext_dir: &Path,
    rootfs: &Path,
    _manifest: &ExtensionManifest,
) -> Result<(), Box<dyn Error>> {
    let manifest_file = ext_dir.join("manifest.yaml");
    copy_tree(ext_dir, rootfs, &manifest_file)?;
    Ok(())
}

fn copy_tree(src: &Path, dest: &Path, skip_file: &Path) -> Result<(), Box<dyn Error>> {
    for entry in WalkDir::new(src).follow_links(false) {
        let entry = entry?;
        let path = entry.path();

        if path == skip_file {
            continue;
        }

        let rel_path = path.strip_prefix(src)?;
        if rel_path.as_os_str().is_empty() {
            continue;
        }

        let dest_path = dest.join(rel_path);

        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&dest_path)?;
        } else if entry.file_type().is_symlink() {
            let link_target = std::fs::read_link(path)?;
            if let Some(parent) = dest_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            #[cfg(unix)]
            {
                let _ = std::fs::remove_file(&dest_path);
                std::os::unix::fs::symlink(&link_target, &dest_path)?;
            }
        } else {
            if let Some(parent) = dest_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(path, &dest_path)?;

            #[cfg(unix)]
            {
                let metadata = path.metadata()?;
                let permissions = metadata.permissions();
                std::fs::set_permissions(&dest_path, permissions)?;
            }
        }
    }

    Ok(())
}
