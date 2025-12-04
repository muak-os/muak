use backhand::{FilesystemCompressor, FilesystemWriter, NodeHeader, compression::Compressor};
use cpio::{NewcBuilder, newc::ModeFileType};
use std::error::Error;
use std::fs::File;
use std::io::{Cursor, Write};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use walkdir::WalkDir;

pub fn squash(src: &Path, dest: &Path) -> Result<(), Box<dyn Error>> {
    let mut writer = FilesystemWriter::default();
    let compressor = FilesystemCompressor::new(Compressor::Zstd, None)?;
    writer.set_compressor(compressor);
    writer.set_block_size(1024 * 1024);

    for entry in WalkDir::new(src).follow_links(false).sort_by_file_name() {
        let entry = entry?;
        let path = entry.path();
        let rel_path = path.strip_prefix(src)?;

        if rel_path.as_os_str().is_empty() {
            continue;
        }

        let path_str = format!("/{}", rel_path.display());
        let metadata = entry.metadata()?;

        #[cfg(unix)]
        let (uid, gid, mode, mtime) = {
            (
                metadata.uid(),
                metadata.gid(),
                metadata.mode(),
                metadata.mtime(),
            )
        };

        #[cfg(not(unix))]
        let (uid, gid, mode, mtime) = (0, 0, 0o644, 0);

        if metadata.is_dir() {
            let header = NodeHeader::new(mode as u16, uid, gid, mtime as u32);
            writer.push_dir(&path_str, header)?;
        } else if metadata.is_symlink() {
            let link_target = std::fs::read_link(path)?;
            let header = NodeHeader::new(0o777, uid, gid, mtime as u32);
            writer.push_symlink(
                link_target.to_string_lossy().into_owned(),
                &path_str,
                header,
            )?;
        } else if metadata.is_file() {
            let contents = std::fs::read(path)?;
            let header = NodeHeader::new(mode as u16, uid, gid, mtime as u32);
            writer.push_file(Cursor::new(contents), &path_str, header)?;
        }
    }

    let mut output = File::create(dest)?;
    writer.write(&mut output)?;

    Ok(())
}

pub fn rebuild_initramfs(src: &Path, dest: &Path) -> Result<(), Box<dyn Error>> {
    let entries: Vec<_> = WalkDir::new(src)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_map(|e| e.ok())
        .collect();

    let mut cpio_data = Vec::new();
    let mut inode_counter = 1u32;

    for entry in entries {
        let path = entry.path();
        let rel_path = path.strip_prefix(src)?;

        let cpio_path = if rel_path.as_os_str().is_empty() {
            ".".to_string()
        } else {
            rel_path.to_string_lossy().to_string()
        };

        let metadata = path.metadata()?;

        #[cfg(unix)]
        let (uid, gid, mode, mtime) = {
            use std::os::unix::fs::MetadataExt;
            (
                metadata.uid(),
                metadata.gid(),
                metadata.mode(),
                metadata.mtime() as u32,
            )
        };

        #[cfg(not(unix))]
        let (uid, gid, mode, mtime) = (0u32, 0u32, 0o644u32, 0u32);

        let mut builder = NewcBuilder::new(&cpio_path)
            .ino(inode_counter)
            .uid(uid)
            .gid(gid)
            .mode(mode)
            .mtime(mtime);

        inode_counter += 1;

        if metadata.is_dir() {
            builder = builder.set_mode_file_type(ModeFileType::Directory);
            let writer = builder.write(&mut cpio_data, 0);
            writer.finish()?;
        } else if metadata.is_symlink() {
            let link_target = std::fs::read_link(path)?;
            let link_str = link_target.to_string_lossy();
            let link_bytes = link_str.as_bytes();

            builder = builder.set_mode_file_type(ModeFileType::Symlink);
            let mut writer = builder.write(&mut cpio_data, link_bytes.len() as u32);
            writer.write_all(link_bytes)?;
            writer.finish()?;
        } else if metadata.is_file() {
            let file_data = std::fs::read(path)?;
            builder = builder.set_mode_file_type(ModeFileType::Regular);
            let mut writer = builder.write(&mut cpio_data, file_data.len() as u32);
            writer.write_all(&file_data)?;
            writer.finish()?;
        }
    }

    cpio::newc::trailer(&mut cpio_data)?;

    let output = File::create(dest)?;
    let mut encoder = zstd::Encoder::new(output, 19)?;
    encoder.write_all(&cpio_data)?;
    encoder.finish()?;

    Ok(())
}
