use std::fs::{File, OpenOptions};
use std::io::{Seek as _, Write as _};

use ::disk::role::Role;
use anyhow::Result;
use parttable::gpt;

use super::blkpg::delete_all_partitions_blkpg;
use super::constants::MB;
use super::guard::Target;
use super::manage::delete_partitions;
use super::mount::{get_disk_mounts, unmount_all};

const ZERO_CHUNK: usize = 16 * 1024;
const HEAD_BYTES: u64 = 10 * MB;
const TAIL_BYTES: u64 = MB;

/// How much of the disk the wipe overwrites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Method {
    /// Zero the head and tail of the disk, destroying the partition table.
    Fast,
    /// Zero the whole disk.
    Zeroes,
}

/// Options controlling a disk wipe.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Options {
    pub method: Method,
    pub drop_partition: bool,
    pub _allow_system_disk: bool,
    pub dry_run: bool,
}

/// Validates the target, releases it and wipes the requested regions.
pub(crate) fn run(target: &Target, options: Options) -> Result<String> {
    let summary = summary(target, options);
    if options.dry_run {
        return Ok(summary);
    }

    if needs_release(target) {
        release(target)?;
    }

    delete_all_partitions_blkpg(&target.path)?;

    if options.drop_partition && !target.partitions.is_empty() {
        drop_entries(target)?;
    }

    write(&target.path, options.method)?;

    Ok(summary)
}

/// Wipes the first and last portions of a disk to remove partition tables.
pub(crate) fn fast(disk: &str) -> Result<()> {
    write(disk, Method::Fast)
}

fn summary(target: &Target, options: Options) -> String {
    let method = match options.method {
        Method::Fast => "fast, head and tail",
        Method::Zeroes => "zeroes, whole disk",
    };
    let drop = if options.drop_partition {
        " and drop its partition entries"
    } else {
        ""
    };

    if options.dry_run {
        format!("Would wipe {} ({method}){drop}", target.path)
    } else {
        format!("Wiped {} ({method}){drop}", target.path)
    }
}

fn needs_release(target: &Target) -> bool {
    !get_disk_mounts(&target.path).is_empty() || target.is_system || target.is_data
}

fn release(target: &Target) -> Result<()> {
    let mounts = get_disk_mounts(&target.path);
    unmount_all(&mounts)?;

    if target.is_system {
        close_mapping(Role::State);
        close_mapping(Role::Ephemeral);
    }
    if target.is_data {
        close_mapping(Role::Data);
    }

    Ok(())
}

fn close_mapping(role: Role) {
    if let Err(e) = luks2::close(role.dm_name()) {
        kmsg::warn!("Failed to close LUKS {} mapping: {}", role.dm_name(), e);
    }
}

fn drop_entries(target: &Target) -> Result<()> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&target.path)?;
    if gpt::io::read(&mut file).is_err() {
        kmsg::info!(
            "No GPT on {}, kernel partitions dropped, wipe will remove any table",
            target.path
        );
        return Ok(());
    }
    drop(file);

    delete_partitions(&target.path, &target.partitions)
}

fn write(disk: &str, method: Method) -> Result<()> {
    let mut file = OpenOptions::new().read(true).write(true).open(disk)?;
    let size = file.seek(std::io::SeekFrom::End(0))?;
    file.seek(std::io::SeekFrom::Start(0))?;

    overwrite(&mut file, size, method)?;
    file.sync_all()?;

    Ok(())
}

fn overwrite(file: &mut File, size: u64, method: Method) -> Result<()> {
    match method {
        Method::Fast => {
            write_zeros(file, HEAD_BYTES.min(size))?;
            if size > TAIL_BYTES {
                file.seek(std::io::SeekFrom::Start(size.saturating_sub(TAIL_BYTES)))?;
                write_zeros(file, TAIL_BYTES)?;
            }
        }
        Method::Zeroes => write_zeros(file, size)?,
    }

    Ok(())
}

fn write_zeros(file: &mut File, mut bytes: u64) -> Result<()> {
    let zeros = [0_u8; ZERO_CHUNK];
    let chunk_limit = u64::try_from(ZERO_CHUNK).unwrap_or(0);

    while bytes > 0 {
        let chunk = usize::try_from(bytes.min(chunk_limit)).unwrap_or(0);
        file.write_all(zeros.get(..chunk).unwrap_or(&zeros))?;
        bytes = bytes.saturating_sub(u64::try_from(chunk).unwrap_or(0));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use tempfile::NamedTempFile;

    use super::*;

    fn target(path: &str) -> Target {
        Target {
            path: path.to_owned(),
            is_system: false,
            is_data: false,
            partitions: Vec::new(),
        }
    }

    fn options(dry_run: bool) -> Options {
        Options {
            method: Method::Fast,
            drop_partition: false,
            _allow_system_disk: false,
            dry_run,
        }
    }

    fn disk(bytes: usize) -> NamedTempFile {
        let mut file = NamedTempFile::new().expect("temp file");
        file.write_all(&vec![0xAA_u8; bytes]).expect("write");
        file
    }

    fn open(file: &NamedTempFile) -> File {
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(file.path())
            .expect("open")
    }

    fn read(file: &NamedTempFile) -> Vec<u8> {
        let mut bytes = Vec::new();
        OpenOptions::new()
            .read(true)
            .open(file.path())
            .expect("open")
            .read_to_end(&mut bytes)
            .expect("read");

        bytes
    }

    #[test]
    fn dry_run_reports_the_target_without_writing() {
        // ARRANGE
        let file = disk(4096);
        let path = file.path().to_str().expect("path");

        // ACT
        let summary = run(&target(path), options(true)).expect("dry run");

        // ASSERT
        assert!(summary.contains("Would wipe"), "{summary}");
        assert!(read(&file).iter().all(|byte| *byte == 0xAA));
    }

    #[test]
    fn zeroes_method_overwrites_the_whole_disk() {
        // ARRANGE
        let file = disk(8192);
        let path = file.path().to_str().expect("path");

        // ACT
        write(path, Method::Zeroes).expect("wipe");

        // ASSERT
        assert!(read(&file).iter().all(|byte| *byte == 0));
    }

    #[test]
    fn overwrite_zeroes_the_given_size() {
        // ARRANGE
        let file = disk(8192);
        let size = 8192_u64;

        // ACT
        overwrite(&mut open(&file), size, Method::Zeroes).expect("wipe");

        // ASSERT
        assert!(read(&file).iter().all(|byte| *byte == 0));
    }

    #[test]
    fn overwrite_writes_nothing_for_a_zero_size() {
        // ARRANGE
        let file = disk(4096);

        // ACT
        overwrite(&mut open(&file), 0, Method::Zeroes).expect("wipe");

        // ASSERT
        assert!(read(&file).iter().all(|byte| *byte == 0xAA));
    }

    #[test]
    fn fast_overwrite_zeroes_the_head_and_tail_of_the_given_size() {
        // ARRANGE
        let size = usize::try_from(12 * MB).expect("size");
        let file = disk(size);
        let limit = u64::try_from(size).expect("limit");

        // ACT
        overwrite(&mut open(&file), limit, Method::Fast).expect("wipe");

        // ASSERT
        let bytes = read(&file);
        let head = usize::try_from(HEAD_BYTES).expect("head");
        let tail = usize::try_from(TAIL_BYTES).expect("tail");
        assert!(bytes[..head].iter().all(|byte| *byte == 0));
        assert!(bytes[size - tail..].iter().all(|byte| *byte == 0));
        assert_eq!(bytes[head], 0xAA);
    }

    #[test]
    fn fast_overwrite_keeps_a_disk_smaller_than_a_megabyte_its_size() {
        // ARRANGE
        let file = disk(4096);

        // ACT
        overwrite(&mut open(&file), 4096, Method::Fast).expect("wipe");

        // ASSERT
        assert_eq!(read(&file).len(), 4096);
    }
}
