use std::fs;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use nix::unistd::sync;

use super::uki::UkiComponents;
use super::{UPDATE_DIR, ValidationMarker, prepare_uki};

pub struct UpdateResult {
    pub update_id: String,
}

pub fn update(version: &str, extensions: &[String]) -> Result<UpdateResult> {
    kmsg::info!(@ "provisioning", "Starting update to version {}", version);

    let staging_dir = create_staging_directory()?;
    let installer_image = format!("ghcr.io/sawangg/installer:{}", version);

    let components = prepare_uki(&installer_image, extensions, &staging_dir)?;

    let marker = create_validation_marker(version)?;
    save_validation_marker(&staging_dir, &marker)?;

    let update_id = marker.update_id.clone();

    if let Err(e) = kexec(&components, &update_id) {
        kmsg::error!(@ "provisioning", "kexec failed: {}", e);
    }

    // We should not reach here if kexec is successful
    Ok(UpdateResult { update_id })
}

fn create_staging_directory() -> Result<PathBuf> {
    let staging_dir = PathBuf::from(UPDATE_DIR);
    fs::create_dir_all(&staging_dir).context("Failed to create update staging dir")?;
    Ok(staging_dir)
}

fn create_validation_marker(target_version: &str) -> Result<ValidationMarker> {
    let current_version = fs::read_to_string("/run/state/VERSION")
        .unwrap_or_default()
        .trim()
        .to_string();

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let update_id = format!("update-{}", timestamp);

    Ok(ValidationMarker {
        update_id,
        target_version: target_version.to_string(),
        current_version,
        timestamp,
    })
}

fn save_validation_marker(staging_dir: &Path, marker: &ValidationMarker) -> Result<()> {
    let marker_json = serde_json::to_string_pretty(marker)?;
    let marker_path = staging_dir.join("pending-validation.json");
    fs::write(marker_path, marker_json).context("Failed to write validation marker")
}

fn kexec(components: &UkiComponents, update_id: &str) -> Result<()> {
    let kernel = fs::File::open(&components.kernel).context("Failed to open kernel for kexec")?;
    let initrd =
        fs::File::open(&components.initramfs).context("Failed to open initramfs for kexec")?;

    let cmdline = prepare_cmdline_with_update_marker(update_id)?;

    load_kernel_into_memory(&kernel, &initrd, &cmdline)?;
    sync();

    nix::sys::reboot::reboot(nix::sys::reboot::RebootMode::RB_KEXEC)
        .map_err(|e| anyhow!("reboot RB_KEXEC failed: {}", e))?;

    Ok(())
}

fn prepare_cmdline_with_update_marker(update_id: &str) -> Result<std::ffi::CString> {
    let base_cmdline = fs::read_to_string("/proc/cmdline")
        .unwrap_or_default()
        .trim()
        .to_string();

    let cmdline_with_marker = format!("{} muak.update_id={}", base_cmdline, update_id);

    std::ffi::CString::new(cmdline_with_marker)
        .map_err(|_| anyhow!("Kernel cmdline contains interior NUL"))
}

fn load_kernel_into_memory(
    kernel: &fs::File,
    initrd: &fs::File,
    cmdline: &std::ffi::CString,
) -> Result<()> {
    let cmdline_ptr = cmdline.as_ptr();
    let cmdline_len = cmdline.as_bytes().len();

    let res = unsafe {
        nix::libc::syscall(
            nix::libc::SYS_kexec_file_load,
            kernel.as_raw_fd(),
            initrd.as_raw_fd(),
            cmdline_len as nix::libc::size_t,
            cmdline_ptr,
            0 as nix::libc::c_ulong,
        )
    };

    if res != 0 {
        return Err(anyhow!("kexec_file_load failed: {}", res));
    }

    Ok(())
}
