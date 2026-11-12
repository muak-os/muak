use std::fs::{File, OpenOptions};
use std::io::{Seek as _, SeekFrom, Write as _};
use std::path::PathBuf;

use anyhow::Result;

use super::WORKLOAD_DIR;

pub(crate) fn create_raw(vm_id: &str, size_bytes: u64) -> Result<PathBuf> {
    let path = get_path(vm_id);

    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)?;

    create_sparse_file(file, size_bytes)?;

    println!(
        "Created raw disk image {} ({} bytes)",
        path.display(),
        size_bytes
    );

    Ok(path)
}

pub(crate) fn get_path(vm_id: &str) -> PathBuf {
    PathBuf::from(WORKLOAD_DIR).join(vm_id).join("disk.raw")
}

fn create_sparse_file(mut file: File, size_bytes: u64) -> Result<()> {
    file.seek(SeekFrom::Start(size_bytes.saturating_sub(1)))?;
    file.write_all(&[0])?;
    file.sync_all()?;

    Ok(())
}
