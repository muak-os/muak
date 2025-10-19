use nix::mount::{mount, MsFlags};
use nix::sys::stat::Mode;
use nix::unistd::mkdir;
use std::path::Path;

// Loop device ioctl constants
const LOOP_SET_FD: u64 = 0x4C00;

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
        "/run",
        "tmpfs",
        "tmpfs",
        MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
        Some("mode=0755"),
    )?;
    create_and_mount(
        "/tmp",
        "tmpfs",
        "tmpfs",
        MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
        Some("mode=1777"),
    )?;

    Ok(())
}

pub fn mount_rootfs() -> Result<(), Box<dyn std::error::Error>> {
    use nix::fcntl::{open, OFlag};
    use nix::unistd::close;

    let newroot = Path::new("/newroot");

    if !newroot.exists() {
        mkdir(newroot, Mode::from_bits_truncate(0o755))?;
    }

    // Open the squashfs file
    let sqsh_fd = open("/rootfs.sqsh", OFlag::O_RDONLY, Mode::empty())?;

    // Find and open a free loop device
    let loop_fd = open("/dev/loop0", OFlag::O_RDWR, Mode::empty())?;

    // Attach the squashfs file to the loop device
    unsafe {
        let ret = nix::libc::ioctl(loop_fd, LOOP_SET_FD, sqsh_fd);
        if ret < 0 {
            close(sqsh_fd).ok();
            close(loop_fd).ok();
            return Err(format!(
                "Failed to attach loop device: errno {}",
                *nix::libc::__errno_location()
            )
            .into());
        }
    }

    // Close file descriptors - kernel keeps the loop device active
    close(sqsh_fd)?;
    close(loop_fd)?;

    // Mount the loop device
    mount(
        Some("/dev/loop0"),
        "/newroot",
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
