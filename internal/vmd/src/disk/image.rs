use anyhow::Result;
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::PathBuf;

use super::DATA_DIR;

pub fn get_image_path(vm_id: &str) -> PathBuf {
    PathBuf::from(DATA_DIR).join(vm_id).join("disk.raw")
}

pub fn create_raw_image(vm_id: &str, size_bytes: u64) -> Result<PathBuf> {
    let path = get_image_path(vm_id);

    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)?;

    create_sparse_file(file, size_bytes)?;

    kmsg::info!(@ "vmd", "Created raw disk image {} ({} bytes)", path.display(), size_bytes);
    Ok(path)
}

fn create_sparse_file(mut file: File, size_bytes: u64) -> Result<()> {
    file.seek(SeekFrom::Start(size_bytes.saturating_sub(1)))?;
    file.write_all(&[0])?;
    file.sync_all()?;
    Ok(())
}
