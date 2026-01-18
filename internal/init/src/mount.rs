use anyhow::{Context, Result};
use rustix::fs::{CWD, Mode, OFlags, mkdirat, open};
use rustix::ioctl::{IntegerSetter, Opcode, ioctl};
use rustix::mount::{MountFlags, mount};
use std::ffi::CString;
use std::os::fd::{AsFd, AsRawFd};
use std::path::Path;

const LOOP_SET_FD: Opcode = 0x4C00;

pub fn mount_pseudo() -> Result<()> {
    create_and_mount(
        "/dev",
        "devtmpfs",
        "devtmpfs",
        MountFlags::NOSUID | MountFlags::NOEXEC,
        None,
    )?;
    create_and_mount(
        "/proc",
        "proc",
        "proc",
        MountFlags::NOSUID | MountFlags::NOEXEC | MountFlags::NODEV,
        None,
    )?;
    create_and_mount(
        "/sys",
        "sysfs",
        "sysfs",
        MountFlags::NOSUID | MountFlags::NOEXEC | MountFlags::NODEV,
        None,
    )?;
    create_and_mount(
        "/run",
        "tmpfs",
        "tmpfs",
        MountFlags::NOSUID | MountFlags::NODEV,
        Some("mode=0755"),
    )?;

    Ok(())
}

pub fn mount_rootfs() -> Result<()> {
    let newroot = Path::new("/newroot");
    if !newroot.exists() {
        mkdirat(CWD, newroot, Mode::from_bits_truncate(0o755))
            .context("Failed to create /newroot")?;
    }

    let work_dir = Path::new("/overlay");
    mkdirat(CWD, work_dir, Mode::from_bits_truncate(0o755)).context("Failed to create /overlay")?;

    let mut lower_dirs = Vec::new();

    let base_mount = work_dir.join("base");
    mkdirat(CWD, &base_mount, Mode::from_bits_truncate(0o755))
        .context("Failed to create /overlay/base")?;
    let base_mount_str = base_mount
        .to_str()
        .context("base mount path contains invalid UTF-8")?;
    attach_squashfs("/rootfs.sqsh", "/dev/loop0", base_mount_str)?;
    lower_dirs.push(base_mount_str.to_string());

    let extensions = discover_extensions();

    if !extensions.is_empty() {
        kmsg::info!("Loading {} extension(s)", extensions.len());
    }

    for (idx, ext_path) in extensions.iter().enumerate() {
        let ext_name = Path::new(ext_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("ext");

        let ext_mount = work_dir.join(ext_name);
        mkdirat(CWD, &ext_mount, Mode::from_bits_truncate(0o755))
            .context("Failed to create extension mount point")?;

        let loop_dev = format!("/dev/loop{}", idx + 1);
        let ext_mount_str = ext_mount
            .to_str()
            .context("extension mount path contains invalid UTF-8")?;
        attach_squashfs(ext_path, &loop_dev, ext_mount_str)?;
        lower_dirs.push(ext_mount_str.to_string());
    }

    if lower_dirs.len() == 1 {
        mount(
            lower_dirs[0].as_str(),
            "/newroot",
            "",
            MountFlags::BIND | MountFlags::RDONLY | MountFlags::NODEV,
            None,
        )
        .context("Failed to bind mount rootfs")?;
    } else {
        let options = format!("lowerdir={}", lower_dirs.join(":"));
        let options_cstr = CString::new(options.as_str()).expect("CString conversion failed");

        mount(
            "overlay",
            "/newroot",
            "overlay",
            MountFlags::RDONLY | MountFlags::NODEV,
            Some(options_cstr.as_c_str()),
        )
        .context("Failed to mount overlay rootfs")?;
    }

    Ok(())
}

fn discover_extensions() -> Vec<String> {
    let extensions_dir = Path::new("/extensions");
    if !extensions_dir.exists() {
        return Vec::new();
    }

    let Ok(entries) = std::fs::read_dir(extensions_dir) else {
        return Vec::new();
    };

    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let is_sqsh = path.extension().and_then(|s| s.to_str()) == Some("sqsh");
            is_sqsh.then(|| path.to_str().map(String::from)).flatten()
        })
        .collect()
}

fn attach_squashfs(sqsh_path: &str, loop_dev: &str, mount_point: &str) -> Result<()> {
    let sqsh_fd = open(sqsh_path, OFlags::RDONLY, Mode::empty())
        .with_context(|| format!("Failed to open squashfs image: {}", sqsh_path))?;
    let loop_fd = open(loop_dev, OFlags::RDWR, Mode::empty())
        .with_context(|| format!("Failed to open loop device: {}", loop_dev))?;

    let fd_number = sqsh_fd.as_fd().as_raw_fd() as usize;

    // SAFETY: ioctl is inherently unsafe, but IntegerSetter ensures proper argument passing
    unsafe {
        ioctl(&loop_fd, IntegerSetter::<LOOP_SET_FD>::new_usize(fd_number))
            .with_context(|| format!("Failed to attach {} to {}", sqsh_path, loop_dev))?;
    }

    mount(loop_dev, mount_point, "squashfs", MountFlags::RDONLY, None)
        .with_context(|| format!("Failed to mount {} to {}", loop_dev, mount_point))?;

    Ok(())
}

fn create_and_mount(
    target: &str,
    source: &str,
    fstype: &str,
    flags: MountFlags,
    data: Option<&str>,
) -> Result<()> {
    let path = Path::new(target);

    if !path.exists() {
        mkdirat(CWD, path, Mode::from_bits_truncate(0o755))
            .with_context(|| format!("Failed to create mount target: {}", target))?;
    }

    let data_cstring = data.map(|s| CString::new(s).expect("CString conversion failed"));

    mount(source, target, fstype, flags, data_cstring.as_deref())
        .with_context(|| format!("Failed to mount {} to {}", source, target))?;

    Ok(())
}
