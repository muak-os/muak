//! GPT-specific constants and helpers.

/// The standard 1 MiB partition alignment in 512-byte sectors.
pub const ALIGN_1_MIB_SECTORS: u64 = 2048;

/// The EFI System Partition type GUID (C12A7328-F81F-11D2-BA4B-00A0C93EC93B).
pub const EFI_GUID: [u8; 16] = [
    0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9, 0x3b,
];

/// Rounds `lba` up to the nearest multiple of `align`.
pub fn align_up_lba(lba: u64, align: u64) -> u64 {
    if lba.is_multiple_of(align) {
        lba
    } else {
        lba + (align - (lba % align))
    }
}

#[cfg(test)]
mod tests {
    use super::{ALIGN_1_MIB_SECTORS, EFI_GUID, align_up_lba};

    #[test]
    fn align_up_lba_keeps_aligned_value() {
        // ARRANGE
        let lba = ALIGN_1_MIB_SECTORS;

        // ACT
        let result = align_up_lba(lba, ALIGN_1_MIB_SECTORS);

        // ASSERT
        assert_eq!(result, ALIGN_1_MIB_SECTORS);
    }

    #[test]
    fn align_up_lba_rounds_unaligned_value() {
        // ARRANGE
        let lba = ALIGN_1_MIB_SECTORS + 1;

        // ACT
        let result = align_up_lba(lba, ALIGN_1_MIB_SECTORS);

        // ASSERT
        assert_eq!(result, ALIGN_1_MIB_SECTORS * 2);
    }

    #[test]
    fn align_up_lba_keeps_zero() {
        // ARRANGE
        let lba = 0;

        // ACT
        let result = align_up_lba(lba, ALIGN_1_MIB_SECTORS);

        // ASSERT
        assert_eq!(result, 0);
    }

    #[test]
    fn align_up_lba_result_is_always_aligned() {
        // ARRANGE
        let cases = [1u64, 100, 2047, 2048, 2049, 4095, 4096, 100_000];

        // ACT / ASSERT
        for lba in cases {
            let result = align_up_lba(lba, ALIGN_1_MIB_SECTORS);
            assert_eq!(result % ALIGN_1_MIB_SECTORS, 0);
            assert!(result >= lba);
        }
    }

    #[test]
    fn efi_guid_matches_uefi_spec_value() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(
            EFI_GUID,
            [
                0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e,
                0xc9, 0x3b,
            ]
        );
    }
}
