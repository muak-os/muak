use backhand::FilesystemReader;
use cpio::NewcReader;
use std::error::Error;
use std::fs::File;
use std::io::{BufReader, Read};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

pub fn extract_initramfs(src: &Path, dest: &Path) -> Result<(), Box<dyn Error>> {
    std::fs::create_dir_all(dest)?;
    let file = File::open(src)?;
    let decoder = zstd::Decoder::new(BufReader::new(file))?;
    extract_cpio_newc(decoder, dest)?;
    Ok(())
}

fn extract_cpio_newc<R: Read>(mut reader: R, dest: &Path) -> Result<(), Box<dyn Error>> {
    const S_IFMT: u32 = 0o170000;
    const S_IFDIR: u32 = 0o040000;
    const S_IFREG: u32 = 0o100000;
    const S_IFLNK: u32 = 0o120000;

    loop {
        let mut cpio_reader = match NewcReader::new(reader) {
            Ok(r) => r,
            Err(_) => break,
        };

        let entry = cpio_reader.entry();
        if entry.is_trailer() {
            break;
        }

        let filename = entry.name().to_string();
        let mode = entry.mode();
        let file_type = mode & S_IFMT;
        let file_size = entry.file_size();
        let path = dest.join(&filename);

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        if file_type == S_IFDIR {
            std::fs::create_dir_all(&path)?;
            reader = cpio_reader.finish()?;
        } else if file_type == S_IFLNK {
            let mut link_target = String::new();
            cpio_reader.read_to_string(&mut link_target)?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(&link_target, &path)?;
            reader = cpio_reader.finish()?;
        } else if file_type == S_IFREG {
            let mut output = File::create(&path)?;
            let mut limited_reader = cpio_reader.take(file_size as u64);
            std::io::copy(&mut limited_reader, &mut output)?;
            reader = limited_reader.into_inner().finish()?;
            #[cfg(unix)]
            {
                let perms = std::fs::Permissions::from_mode(mode);
                std::fs::set_permissions(&path, perms)?;
            }
        } else {
            reader = cpio_reader.finish()?;
        }
    }

    Ok(())
}

pub fn unsquash(src: &Path, dest: &Path) -> Result<(), Box<dyn Error>> {
    std::fs::create_dir_all(dest)?;

    let file = File::open(src)?;
    let reader = BufReader::new(file);
    let fs = FilesystemReader::from_reader(reader)?;

    for node in fs.files() {
        let path_str = node.fullpath.to_string_lossy();
        let dest_path = dest.join(path_str.strip_prefix('/').unwrap_or(&path_str));

        if let Some(parent) = dest_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        match &node.inner {
            backhand::InnerNode::Dir(_) => {
                std::fs::create_dir_all(&dest_path)?;
            }
            backhand::InnerNode::File(file_node) => {
                let mut output = File::create(&dest_path)?;
                let mut reader = fs.file(file_node).reader();
                std::io::copy(&mut reader, &mut output)?;

                #[cfg(unix)]
                {
                    let mode = node.header.permissions as u32;
                    let perms = std::fs::Permissions::from_mode(mode);
                    std::fs::set_permissions(&dest_path, perms)?;
                }
            }
            backhand::InnerNode::Symlink(symlink) => {
                #[cfg(unix)]
                std::os::unix::fs::symlink(&symlink.link, &dest_path)?;
            }
            _ => {}
        }
    }

    Ok(())
}
