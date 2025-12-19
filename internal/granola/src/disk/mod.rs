// Most disk operations are used by install/update which will be called from grpcd
// Only mount_partitions is currently used from main.rs
#![allow(dead_code)]

mod blkpg;
mod constants;
mod format;
mod gpt;
mod mount;
mod sysfs;
mod types;
mod utils;

pub use blkpg::delete_all_partitions_blkpg;
pub use format::{format_btrfs_partition, format_efi_partition};
pub use gpt::{create_partitions, has_existing_partitions};
pub use mount::mount_partitions;
pub use sysfs::find_partition_by_partname;
#[allow(unused_imports)]
pub use sysfs::{list_disks, validate_block_device, validate_disk_size};
pub use utils::{get_disk_mounts, unmount_all, wipe_disk};
