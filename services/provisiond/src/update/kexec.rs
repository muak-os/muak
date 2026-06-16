//! Kexec-based reboot into the staged update kernel.

use std::fs::{self, File};
use std::os::fd::AsRawFd;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use rustix::fs::sync;
use rustix::system::{RebootCommand, reboot};

use crate::constants::UPDATE_DIR;

#[cfg(target_arch = "x86_64")]
const SYS_KEXEC_FILE_LOAD: libc::c_long = 320;

#[cfg(target_arch = "aarch64")]
const SYS_KEXEC_FILE_LOAD: libc::c_long = 294;

/// Loads the staged kernel via kexec and reboots into it.
pub fn run(update_id: &str) -> Result<()> {
    let update_dir = Path::new(UPDATE_DIR);
    let kernel_path = update_dir.join("assets").join("kernel");
    let initrd_path = update_dir.join("assets").join("initramfs");

    load(&kernel_path, &initrd_path, update_id).context("Failed to load new kernel with kexec")?;
    kmsg::info!("kexec booting into update {}", update_id);
    reboot(RebootCommand::Kexec).map_err(|e| anyhow!("Failed to execute new kernel: {}", e))?;

    unreachable!("If we reach here, something went really wrong")
}

fn load(kernel_path: &Path, initrd_path: &Path, update_id: &str) -> Result<()> {
    let kernel = File::open(kernel_path).context("Failed to open kernel for kexec")?;
    let initrd = File::open(initrd_path).context("Failed to open initrd for kexec")?;
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

    Ok(())
}

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
