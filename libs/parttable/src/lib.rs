//! Shared MBR and GPT partition table helpers.

mod gpt;
mod mbr;

pub use gpt::{ALIGN_1_MIB_SECTORS, EFI_GUID, align_up_lba};
pub use mbr::{
    MBR_BOOT_SIGNATURE, MBR_PARTITION_ENTRY_OFFSET, MBR_PROTECTIVE_GPT_TYPE,
    protective_mbr_size_lba, write_gpt_protective_mbr,
};
