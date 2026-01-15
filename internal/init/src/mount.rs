use anyhow::{Context, Result, bail};
use nix::fcntl::{OFlag, open};
use nix::mount::{MsFlags, mount};
use nix::sys::stat::Mode;
use nix::unistd::{close, mkdir};
use std::os::fd::AsRawFd;
use std::path::Path;

nix::ioctl_write_int_bad!(loop_set_fd, 0x4C00);

pub fn mount_pseudo() -> Result<()> {
    create_and_mount(
        "/dev",
        "devtmpfs",
        "devtmpfs",
        MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC,
        None,
    )?;
    create_and_mount(
        "/proc",
        "proc",
        "proc",
        MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_NODEV,
        None,
    )?;
    create_and_mount(
        "/sys",
        "sysfs",
        "sysfs",
        MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_NODEV,
        None,
    )?;
    create_and_mount(
        "/run",
        "tmpfs",
        "tmpfs",
        MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
        Some("mode=0755"),
    )?;

    Ok(())
}

pub fn mount_rootfs() -> Result<()> {
    let newroot = Path::new("/newroot");
    if !newroot.exists() {
        mkdir(newroot, Mode::from_bits_truncate(0o755)).context("Failed to create /newroot")?;
    }

    let work_dir = Path::new("/overlay");
    mkdir(work_dir, Mode::from_bits_truncate(0o755)).context("Failed to create /overlay")?;

    let mut lower_dirs = Vec::new();

    let base_mount = work_dir.join("base");
    mkdir(&base_mount, Mode::from_bits_truncate(0o755))
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
        mkdir(&ext_mount, Mode::from_bits_truncate(0o755))
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
            Some(lower_dirs[0].as_str()),
            "/newroot",
            None::<&str>,
            MsFlags::MS_BIND | MsFlags::MS_RDONLY | MsFlags::MS_NODEV,
            None::<&str>,
        )
        .context("Failed to bind mount rootfs")?;
    } else {
        let lowerdir = lower_dirs.join(":");
        let options = format!("lowerdir={}", lowerdir);

        mount(
            Some("overlay"),
            "/newroot",
            Some("overlay"),
            MsFlags::MS_RDONLY | MsFlags::MS_NODEV,
            Some(options.as_str()),
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
    let sqsh_fd = open(sqsh_path, OFlag::O_RDONLY, Mode::empty())
        .with_context(|| format!("Failed to open squashfs image: {}", sqsh_path))?;
    let loop_fd = open(loop_dev, OFlag::O_RDWR, Mode::empty())
        .with_context(|| format!("Failed to open loop device: {}", loop_dev))?;

    let result = unsafe { loop_set_fd(loop_fd.as_raw_fd(), sqsh_fd.as_raw_fd()) };

    if result.is_err() {
        close(sqsh_fd).ok();
        close(loop_fd).ok();
        bail!("Failed to attach {} to {}", sqsh_path, loop_dev);
    }

    close(sqsh_fd)?;
    close(loop_fd)?;

    mount(
        Some(loop_dev),
        mount_point,
        Some("squashfs"),
        MsFlags::MS_RDONLY,
        None::<&str>,
    )
    .with_context(|| format!("Failed to mount {} to {}", loop_dev, mount_point))?;

    Ok(())
}

fn create_and_mount(
    target: &str,
    source: &str,
    fstype: &str,
    flags: MsFlags,
    data: Option<&str>,
) -> Result<()> {
    let path = Path::new(target);

    if !path.exists() {
        mkdir(path, Mode::from_bits_truncate(0o755))
            .with_context(|| format!("Failed to create mount target: {}", target))?;
    }

    mount(Some(source), target, Some(fstype), flags, data)
        .with_context(|| format!("Failed to mount {} to {}", source, target))?;

    Ok(())
}
