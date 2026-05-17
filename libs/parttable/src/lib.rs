//! Shared MBR and GPT partition table helpers.

mod gpt;
mod mbr;

pub use gpt::{
    ALIGN_1_MIB_SECTORS, EFI_GUID, GptError, LINUX_FS_GUID, Partition, Table, align_up_lba,
};
pub use mbr::{
    MBR_BOOT_SIGNATURE, MBR_EFI_SYSTEM_TYPE, MBR_PARTITION_ENTRY_OFFSET, MBR_PROTECTIVE_GPT_TYPE,
    MbrPartitionEntry, is_mbr_disk, protective_mbr_size_lba, write_gpt_protective_mbr,
    write_mbr_boot_signature, write_mbr_partition_entry,
};
