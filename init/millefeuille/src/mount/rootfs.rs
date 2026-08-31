//! Mount the root filesystem with layer images as overlays.

use alloc::ffi::CString;
use std::path::Path;

use anyhow::{Context as _, Result};
use rustix::fs::{CWD, Mode, mkdirat};
use rustix::mount::{
    FsMountFlags, FsOpenFlags, MountAttrFlags, MountFlags, MoveMountFlags, fsconfig_create,
    fsconfig_set_string, fsmount, fsopen, mount, move_mount,
};

use super::layers;
use super::{IMAGE_EXTENSION, IMAGE_FSTYPE};

/// Mount the root filesystem with layer images as overlays.
pub(crate) fn rootfs() -> Result<()> {
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
    let base_image = format!("/rootfs.{IMAGE_EXTENSION}");
    mount_image_file(&base_image, base_mount_str).context("Failed to mount base rootfs")?;
    lower_dirs.push(base_mount_str.to_owned());

    let found = layers::discover_layers();

    if found.is_empty() {
        kmsg::info!("No layer images found, using base rootfs only");
    } else {
        kmsg::info!("Loading {} layer(s)", found.len());
    }

    for layer_path in &found {
        let layer_mount = work_dir.join(layer_mount_name(layer_path));
        mkdirat(CWD, &layer_mount, Mode::from_bits_truncate(0o755))
            .context("Failed to create layer mount point")?;

        let layer_mount_str = layer_mount
            .to_str()
            .context("layer mount path contains invalid UTF-8")?;
        mount_image_file(layer_path, layer_mount_str).context("Failed to mount layer rootfs")?;
        lower_dirs.push(layer_mount_str.to_owned());
    }

    if let Some((base, rest)) = lower_dirs.split_first() {
        if rest.is_empty() {
            mount(
                base.as_str(),
                "/newroot",
                "",
                MountFlags::BIND | MountFlags::RDONLY | MountFlags::NODEV,
                None,
            )
            .context("Failed to bind mount rootfs")?;
        } else {
            let options = format!("lowerdir={}", lower_dirs.join(":"));
            let options_cstr =
                CString::new(options.as_str()).context("CString conversion failed")?;

            mount(
                "overlay",
                "/newroot",
                "overlay",
                MountFlags::RDONLY | MountFlags::NODEV,
                Some(options_cstr.as_c_str()),
            )
            .context("Failed to mount overlay rootfs")?;
        }
    }

    Ok(())
}

fn layer_mount_name(layer_path: &str) -> String {
    let name = layer_path.trim_start_matches('/').replace('/', "-");
    if name.is_empty() {
        "layer".to_owned()
    } else {
        name
    }
}

fn mount_image_file(image_path: &str, target: &str) -> Result<()> {
    let fs_fd =
        fsopen(IMAGE_FSTYPE, FsOpenFlags::empty()).context("Failed to fsopen image filesystem")?;
    fsconfig_set_string(&fs_fd, "source", image_path).context("Failed to fsconfig source")?;
    fsconfig_create(&fs_fd).context("Failed to fsconfig_create")?;
    let mnt_fd = fsmount(
        &fs_fd,
        FsMountFlags::empty(),
        MountAttrFlags::MOUNT_ATTR_RDONLY,
    )
    .context("Failed to fsmount")?;

    move_mount(
        &mnt_fd,
        "",
        CWD,
        target,
        MoveMountFlags::MOVE_MOUNT_F_EMPTY_PATH,
    )
    .with_context(|| format!("Failed to move_mount to {target}"))
}
