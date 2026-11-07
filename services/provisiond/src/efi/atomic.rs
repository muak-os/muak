//! Atomic replacement of files on the EFI System Partition.

use std::fs;
use std::io::{self, Read, Write as _};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use esp::path::validate_relative;
use rustix::fs::statvfs;

const NEW_SUFFIX: &str = ".new";
const OLD_SUFFIX: &str = ".old";
const HEADROOM: u64 = 1024 * 1024;

pub(super) fn write_file(
    esp_root: &Path,
    relative_path: &str,
    size: u64,
    reader: &mut dyn Read,
) -> Result<()> {
    validate_relative(relative_path)?;
    let dest = esp_root.join(relative_path);
    super::create_parents(&dest)?;
    let new = suffixed(&dest, NEW_SUFFIX);
    let old = suffixed(&dest, OLD_SUFFIX);

    recover(&dest, &old)?;
    remove_if_exists(&new)?;
    remove_if_exists(&old)?;
    preflight(esp_root, size)?;

    if let Err(error) = stream(&new, size, reader, relative_path) {
        let _discarded = remove_if_exists(&new);
        return Err(error);
    }

    activate(&new, &dest, &old)?;
    if let Some(parent) = dest.parent() {
        sync_dir(parent);
    }

    Ok(())
}

pub(super) fn write_bytes(esp_root: &Path, relative_path: &str, data: &[u8]) -> Result<()> {
    validate_relative(relative_path)?;
    let dest = esp_root.join(relative_path);
    super::create_parents(&dest)?;
    let new = suffixed(&dest, NEW_SUFFIX);

    remove_if_exists(&new)?;
    preflight(esp_root, u64::try_from(data.len()).unwrap_or(u64::MAX))?;

    if let Err(error) = write_all(&new, data) {
        let _discarded = remove_if_exists(&new);
        return Err(error);
    }

    fs::rename(&new, &dest).with_context(|| format!("Failed to activate {}", dest.display()))?;
    if let Some(parent) = dest.parent() {
        sync_dir(parent);
    }

    Ok(())
}

fn stream(dest: &Path, size: u64, reader: &mut dyn Read, label: &str) -> Result<()> {
    let mut writer =
        fs::File::create(dest).with_context(|| format!("Failed to create {}", dest.display()))?;
    let mut buf = [0_u8; 8192];
    let mut remaining = size;
    while remaining > 0 {
        let chunk = usize::try_from(remaining)
            .unwrap_or(usize::MAX)
            .min(buf.len());
        let read = reader
            .read(buf.get_mut(..chunk).unwrap_or(&mut []))
            .context("Failed to read file data")?;
        if read == 0 {
            bail!("reader returned EOF before declared size: {label}");
        }
        writer
            .write_all(buf.get(..read).unwrap_or(&[]))
            .context("Failed to write file to EFI partition")?;
        remaining = remaining.saturating_sub(u64::try_from(read).unwrap_or(0));
    }
    writer
        .flush()
        .context("Failed to flush file to EFI partition")?;
    writer
        .sync_all()
        .with_context(|| format!("Failed to fsync {}", dest.display()))?;

    Ok(())
}

fn write_all(dest: &Path, data: &[u8]) -> Result<()> {
    let mut writer =
        fs::File::create(dest).with_context(|| format!("Failed to create {}", dest.display()))?;
    writer
        .write_all(data)
        .context("Failed to write file to EFI partition")?;
    writer
        .flush()
        .context("Failed to flush file to EFI partition")?;
    writer
        .sync_all()
        .with_context(|| format!("Failed to fsync {}", dest.display()))?;

    Ok(())
}

fn activate(new: &Path, dest: &Path, old: &Path) -> Result<()> {
    if !dest.exists() {
        return fs::rename(new, dest)
            .with_context(|| format!("Failed to activate {}", dest.display()));
    }

    remove_if_exists(old)?;
    fs::rename(dest, old)
        .with_context(|| format!("Failed to back up {} to {}", dest.display(), old.display()))?;
    if let Err(error) = fs::rename(new, dest) {
        let _restored = fs::rename(old, dest);
        return Err(error).with_context(|| format!("Failed to activate {}", dest.display()));
    }

    Ok(())
}

fn recover(dest: &Path, old: &Path) -> Result<()> {
    if dest.exists() || !old.exists() {
        return Ok(());
    }

    fs::rename(old, dest).with_context(|| {
        format!(
            "Failed to restore missing {} from {}",
            dest.display(),
            old.display()
        )
    })?;
    kmsg::warn!(
        "Restored missing EFI boot file {} from {}",
        dest.display(),
        old.display()
    );

    Ok(())
}

fn preflight(esp_root: &Path, required: u64) -> Result<()> {
    let stat =
        statvfs(esp_root).with_context(|| format!("Failed to stat {}", esp_root.display()))?;
    let block = if stat.f_frsize > 0 {
        stat.f_frsize
    } else {
        stat.f_bsize
    };
    let free = stat.f_bavail.saturating_mul(block);
    let needed = required.saturating_add(HEADROOM);
    if free < needed {
        bail!(
            "Not enough free space on {}: need {} bytes, {} available",
            esp_root.display(),
            needed,
            free
        );
    }

    Ok(())
}

fn suffixed(dest: &Path, suffix: &str) -> PathBuf {
    let mut name = dest.as_os_str().to_os_string();
    name.push(suffix);

    PathBuf::from(name)
}

fn remove_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("Failed to remove {}", path.display())),
    }
}

fn sync_dir(dir: &Path) {
    if let Ok(handle) = fs::File::open(dir) {
        let _synced = handle.sync_all();
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Cursor;

    use super::*;

    const REL: &str = "EFI/BOOT/BOOTX64.EFI";

    fn prepare(dest: &Path) {
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).expect("boot directory must be created");
        }
    }

    #[test]
    fn replaces_live_file_and_keeps_backup() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("temp dir must be created");
        let dest = dir.path().join(REL);
        prepare(&dest);
        fs::write(&dest, b"old").expect("old boot file must be written");
        let data = b"new-uki";
        let mut reader = Cursor::new(data.as_slice());

        // ACT
        write_file(
            dir.path(),
            REL,
            u64::try_from(data.len()).unwrap_or(0),
            &mut reader,
        )
        .expect("atomic write must succeed");

        // ASSERT
        assert_eq!(fs::read(&dest).expect("live file must exist"), data);
        assert_eq!(
            fs::read(suffixed(&dest, OLD_SUFFIX)).expect("backup must exist"),
            b"old"
        );
        assert!(!suffixed(&dest, NEW_SUFFIX).exists(), "temp must be gone");
    }

    #[test]
    fn replaces_stale_backup_with_the_previous_live_file() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("temp dir must be created");
        let dest = dir.path().join(REL);
        prepare(&dest);
        fs::write(&dest, b"live").expect("live boot file must be written");
        fs::write(suffixed(&dest, OLD_SUFFIX), b"stale").expect("stale backup must be written");
        let data = b"new";
        let mut reader = Cursor::new(data.as_slice());

        // ACT
        write_file(
            dir.path(),
            REL,
            u64::try_from(data.len()).unwrap_or(0),
            &mut reader,
        )
        .expect("atomic write must succeed");

        // ASSERT
        assert_eq!(fs::read(&dest).expect("live file must exist"), data);
        assert_eq!(
            fs::read(suffixed(&dest, OLD_SUFFIX)).expect("backup must exist"),
            b"live",
            "the stale backup must be replaced by the previous live file"
        );
    }

    #[test]
    fn creates_live_file_without_backup() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("temp dir must be created");
        let dest = dir.path().join(REL);
        prepare(&dest);
        let data = b"first-uki";
        let mut reader = Cursor::new(data.as_slice());

        // ACT
        write_file(
            dir.path(),
            REL,
            u64::try_from(data.len()).unwrap_or(0),
            &mut reader,
        )
        .expect("atomic write must succeed");

        // ASSERT
        assert_eq!(fs::read(&dest).expect("live file must exist"), data);
        assert!(
            !suffixed(&dest, OLD_SUFFIX).exists(),
            "no backup without a previous file"
        );
    }

    #[test]
    fn early_eof_keeps_live_file_and_removes_temp() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("temp dir must be created");
        let dest = dir.path().join(REL);
        prepare(&dest);
        fs::write(&dest, b"old").expect("old boot file must be written");
        let mut reader = Cursor::new(b"short".as_slice());

        // ACT
        let result = write_file(dir.path(), REL, 100, &mut reader);

        // ASSERT
        assert!(result.is_err(), "short input must fail");
        assert_eq!(
            fs::read(&dest).expect("live file must survive"),
            b"old",
            "live file must stay untouched"
        );
        assert!(!suffixed(&dest, NEW_SUFFIX).exists(), "temp must be gone");
    }

    #[test]
    fn restores_missing_live_file_from_backup() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("temp dir must be created");
        let dest = dir.path().join(REL);
        prepare(&dest);
        let old = suffixed(&dest, OLD_SUFFIX);
        fs::write(&old, b"old").expect("backup must be written");
        let data = b"new";
        let mut reader = Cursor::new(data.as_slice());

        // ACT
        write_file(
            dir.path(),
            REL,
            u64::try_from(data.len()).unwrap_or(0),
            &mut reader,
        )
        .expect("atomic write must succeed");

        // ASSERT
        assert_eq!(fs::read(&dest).expect("live file must exist"), data);
        assert_eq!(
            fs::read(&old).expect("backup must exist"),
            b"old",
            "the recovered file becomes the new backup"
        );
    }

    #[test]
    fn preflight_failure_keeps_live_file() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("temp dir must be created");
        let dest = dir.path().join(REL);
        prepare(&dest);
        fs::write(&dest, b"old").expect("old boot file must be written");
        let mut reader = Cursor::new(b"new".as_slice());

        // ACT
        let result = write_file(dir.path(), REL, u64::MAX, &mut reader);

        // ASSERT
        assert!(result.is_err(), "an impossible size must fail preflight");
        assert_eq!(
            fs::read(&dest).expect("live file must survive"),
            b"old",
            "the live file must stay untouched"
        );
        assert!(!suffixed(&dest, NEW_SUFFIX).exists(), "temp must be gone");
    }

    #[test]
    fn rejects_writes_larger_than_free_space() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("temp dir must be created");

        // ACT
        let result = preflight(dir.path(), u64::MAX);

        // ASSERT
        assert!(result.is_err(), "impossible writes must fail preflight");
    }

    #[test]
    fn accepts_writes_that_fit() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("temp dir must be created");

        // ACT
        let result = preflight(dir.path(), 4096);

        // ASSERT
        result.expect("a small write must fit");
    }

    #[test]
    fn write_bytes_replaces_content() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("temp dir must be created");
        let dest = dir.path().join("luks");
        fs::write(&dest, b"old-key").expect("old key must be written");

        // ACT
        write_bytes(dir.path(), "luks", b"new-key").expect("atomic write must succeed");

        // ASSERT
        assert_eq!(fs::read(&dest).expect("key file must exist"), b"new-key");
        assert!(!suffixed(&dest, NEW_SUFFIX).exists(), "temp must be gone");
    }

    #[test]
    fn rejects_parent_traversal() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("temp dir must be created");

        // ACT
        let result = write_bytes(dir.path(), "../escape", b"data");

        // ASSERT
        assert!(result.is_err(), "traversal must be rejected");
    }
}
