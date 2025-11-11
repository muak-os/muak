use nix::fcntl::{OFlag, open};
use nix::mount::{MsFlags, mount};
use nix::sys::stat::Mode;
use nix::unistd::close;
use nix::unistd::mkdir;
use serde::Deserialize;
use std::path::Path;

const LOOP_SET_FD: u64 = 0x4C00;

#[derive(Debug, Deserialize)]
struct ExtensionManifest {
    #[serde(default)]
    extensions: Vec<Extension>,
}

#[derive(Debug, Deserialize)]
struct Extension {
    name: String,
    file: String,
}

pub fn mount_pseudo() -> Result<(), Box<dyn std::error::Error>> {
    create_and_mount("/dev", "devtmpfs", "devtmpfs", MsFlags::MS_NOSUID, None)?;
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
        "/tmp",
        "tmpfs",
        "tmpfs",
        MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
        Some("mode=1777"),
    )?;
    create_and_mount(
        "/mnt",
        "tmpfs",
        "tmpfs",
        MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
        Some("mode=0755"),
    )?;

    Ok(())
}

pub fn mount_rootfs() -> Result<(), Box<dyn std::error::Error>> {
    let newroot = Path::new("/newroot");
    if !newroot.exists() {
        mkdir(newroot, Mode::from_bits_truncate(0o755))?;
    }

    let manifest = read_extensions_manifest()?;

    let work_dir = Path::new("/overlay");
    mkdir(work_dir, Mode::from_bits_truncate(0o755))?;

    let mut lower_dirs = Vec::new();

    let base_mount = work_dir.join("base");
    mkdir(&base_mount, Mode::from_bits_truncate(0o755))?;
    attach_squashfs("/rootfs.sqsh", "/dev/loop0", base_mount.to_str().unwrap())?;
    lower_dirs.push(base_mount.to_str().unwrap().to_string());

    if !manifest.extensions.is_empty() {
        crate::logging::log(&format!(
            "Loading {} extension(s)",
            manifest.extensions.len()
        ));
    }

    for (idx, ext) in manifest.extensions.iter().enumerate() {
        let ext_mount = work_dir.join(&ext.name);
        mkdir(&ext_mount, Mode::from_bits_truncate(0o755))?;

        let ext_path = format!("/{}", ext.file);
        let loop_dev = format!("/dev/loop{}", idx + 1);
        attach_squashfs(&ext_path, &loop_dev, ext_mount.to_str().unwrap())?;
        lower_dirs.push(ext_mount.to_str().unwrap().to_string());
    }

    if lower_dirs.len() == 1 {
        mount(
            Some(lower_dirs[0].as_str()),
            "/newroot",
            None::<&str>,
            MsFlags::MS_BIND | MsFlags::MS_RDONLY,
            None::<&str>,
        )?;
    } else {
        let lowerdir = lower_dirs.join(":");
        let options = format!("lowerdir={}", lowerdir);

        mount(
            Some("overlay"),
            "/newroot",
            Some("overlay"),
            MsFlags::MS_RDONLY,
            Some(options.as_str()),
        )?;
    }

    Ok(())
}

fn read_extensions_manifest() -> Result<ExtensionManifest, Box<dyn std::error::Error>> {
    let manifest_path = "/extensions.yaml";
    if !Path::new(manifest_path).exists() {
        return Ok(ExtensionManifest { extensions: vec![] });
    }

    let content = std::fs::read_to_string(manifest_path)?;
    let manifest: ExtensionManifest = serde_yaml::from_str(&content)?;
    Ok(manifest)
}

fn attach_squashfs(
    sqsh_path: &str,
    loop_dev: &str,
    mount_point: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let sqsh_fd = open(sqsh_path, OFlag::O_RDONLY, Mode::empty())?;
    let loop_fd = open(loop_dev, OFlag::O_RDWR, Mode::empty())?;

    unsafe {
        let ret = nix::libc::ioctl(loop_fd, LOOP_SET_FD, sqsh_fd);
        if ret < 0 {
            close(sqsh_fd).ok();
            close(loop_fd).ok();
            return Err(format!("Failed to attach {} to {}", sqsh_path, loop_dev).into());
        }
    }

    close(sqsh_fd)?;
    close(loop_fd)?;

    mount(
        Some(loop_dev),
        mount_point,
        Some("squashfs"),
        MsFlags::MS_RDONLY,
        None::<&str>,
    )?;

    Ok(())
}

fn create_and_mount(
    target: &str,
    source: &str,
    fstype: &str,
    flags: MsFlags,
    data: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = Path::new(target);

    if !path.exists() {
        mkdir(path, Mode::from_bits_truncate(0o755))?;
    }

    mount(Some(source), target, Some(fstype), flags, data)?;

    Ok(())
}
