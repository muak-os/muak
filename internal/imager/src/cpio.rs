use backhand::{FilesystemCompressor, FilesystemWriter, NodeHeader, compression::Compressor};
use cpio::{NewcBuilder, newc::ModeFileType};
use std::io::{Cursor, Write};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use walkdir::WalkDir;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub fn create_squashfs_from_directory(source_dir: &Path) -> Result<Vec<u8>> {
    let mut writer = FilesystemWriter::default();
    let compressor = FilesystemCompressor::new(Compressor::Zstd, None)?;
    writer.set_compressor(compressor);
    writer.set_block_size(1024 * 1024);

    for entry in WalkDir::new(source_dir).follow_links(false).sort_by_file_name() {
        let entry = entry?;
        let path = entry.path();
        let rel_path = path.strip_prefix(source_dir)?;

        if rel_path.as_os_str().is_empty() {
            continue;
        }

        let archive_path = format!("/{}", rel_path.display());
        let metadata = entry.metadata()?;

        let (uid, gid, mode, mtime) = (
            metadata.uid(),
            metadata.gid(),
            metadata.mode(),
            metadata.mtime(),
        );

        if metadata.is_dir() {
            let header = NodeHeader::new(mode as u16, uid, gid, mtime as u32);
            writer.push_dir(&archive_path, header)?;
        } else if metadata.is_symlink() {
            let link_target = std::fs::read_link(path)?;
            let header = NodeHeader::new(0o777, uid, gid, mtime as u32);
            writer.push_symlink(
                link_target.to_string_lossy().into_owned(),
                &archive_path,
                header,
            )?;
        } else if metadata.is_file() {
            let contents = std::fs::read(path)?;
            let header = NodeHeader::new(mode as u16, uid, gid, mtime as u32);
            writer.push_file(Cursor::new(contents), &archive_path, header)?;
        }
    }

    let mut output = Cursor::new(Vec::new());
    writer.write(&mut output)?;
    Ok(output.into_inner())
}

pub fn create_cpio_archive(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    let mut cpio_data = Vec::new();
    let mut inode = 1u32;

    let dir_builder = NewcBuilder::new("extensions")
        .ino(inode)
        .uid(0)
        .gid(0)
        .mode(0o755)
        .set_mode_file_type(ModeFileType::Directory);
    inode += 1;
    let writer = dir_builder.write(&mut cpio_data, 0);
    writer.finish()?;

    for (path, data) in files {
        let builder = NewcBuilder::new(path)
            .ino(inode)
            .uid(0)
            .gid(0)
            .mode(0o644)
            .set_mode_file_type(ModeFileType::Regular);
        inode += 1;

        let mut writer = builder.write(&mut cpio_data, data.len() as u32);
        writer.write_all(data)?;
        writer.finish()?;
    }

    cpio::newc::trailer(&mut cpio_data)?;

    Ok(cpio_data)
}
