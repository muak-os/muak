//! Update preparation and kexec execution for atomic system updates.

use std::fs;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use rustix::fs::sync;
use rustix::system::{RebootCommand, reboot};

use crate::constants::UPDATE_DIR;
use crate::uki::Uki;
use crate::validation::ValidationMarker;

#[cfg(target_arch = "x86_64")]
const SYS_KEXEC_FILE_LOAD: libc::c_long = 320;

#[cfg(target_arch = "aarch64")]
const SYS_KEXEC_FILE_LOAD: libc::c_long = 294;

/// Prepares an update by staging the UKI components.
pub fn prepare(image: &str, extensions: &[String]) -> Result<String> {
    let staging_dir = create_staging_directory()?;
    Uki::prepare(image, extensions, &staging_dir)?;

    let marker = create_validation_marker(image)?;
    save_validation_marker(&staging_dir, &marker)?;

    sync();

    Ok(marker.update_id)
}

/// Executes the update by loading the new kernel via kexec.
pub fn update(update_id: &str) -> Result<()> {
    let uki = Uki::from_dir(Path::new(UPDATE_DIR));
    kexec(&uki, update_id)
}

/// Creates the staging directory for update files.
fn create_staging_directory() -> Result<PathBuf> {
    let staging_dir = PathBuf::from(UPDATE_DIR);
    fs::create_dir_all(&staging_dir).context("Failed to create update staging dir")?;
    Ok(staging_dir)
}

/// Creates a validation marker for tracking the update.
fn create_validation_marker(target_image: &str) -> Result<ValidationMarker> {
    let current_image = sysconfig::system().image.clone();

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let update_id = format!("update-{}", timestamp);

    Ok(ValidationMarker {
        update_id,
        target_image: target_image.to_string(),
        current_image,
        timestamp,
    })
}

/// Saves the validation marker to disk.
fn save_validation_marker(staging_dir: &Path, marker: &ValidationMarker) -> Result<()> {
    let marker_json = serde_json::to_string_pretty(marker)?;
    let marker_path = staging_dir.join("pending-validation.json");
    fs::write(marker_path, marker_json).context("Failed to write validation marker")
}

/// Loads the new kernel via kexec and reboots into it.
fn kexec(uki: &Uki, update_id: &str) -> Result<()> {
    let kernel = fs::File::open(&uki.kernel).context("Failed to open kernel for kexec")?;
    let initrd = fs::File::open(&uki.initramfs).context("Failed to open initramfs for kexec")?;
    let cmdline = add_cmdline_update_marker(update_id)?;

    // SAFETY: kexec_file_load syscall with valid file descriptors and null-terminated string
    let res = unsafe {
        libc::syscall(
            SYS_KEXEC_FILE_LOAD,
            kernel.as_raw_fd(),
            initrd.as_raw_fd(),
            cmdline.as_bytes_with_nul().len() as libc::size_t,
            cmdline.as_ptr(),
            0 as libc::c_ulong,
        )
    };

    if res != 0 {
        let errno = std::io::Error::last_os_error();
        return Err(anyhow!("Failed to load new kernel: {}", errno));
    }

    sync();

    reboot(RebootCommand::Kexec).map_err(|e| anyhow!("Failed to execute new kernel: {}", e))?;

    unreachable!("If we reach here, something went really wrong");
}

/// Adds the update ID marker to the kernel cmdline.
fn add_cmdline_update_marker(update_id: &str) -> Result<std::ffi::CString> {
    let cmdline = fs::read_to_string("/proc/cmdline")
        .unwrap_or_default()
        .trim()
        .to_string();

    let filtered_cmdline = cmdline
        .split_whitespace()
        .filter(|arg| !arg.starts_with("muak.update_id="))
        .collect::<Vec<_>>()
        .join(" ");

    std::ffi::CString::new(format!("{} muak.update_id={}", filtered_cmdline, update_id))
        .map_err(|_| anyhow!("Kernel cmdline contains interior NUL"))
}
