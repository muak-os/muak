//! GPT partition entry wire format and partition type GUIDs.

/// Linux filesystem partition type GUID (0FC63DAF-8483-4772-8E79-3D69D8477DE4).
pub const LINUX_FS_GUID: [u8; 16] = [
    0xaf, 0x3d, 0xc6, 0x0f, 0x83, 0x84, 0x72, 0x47, 0x8e, 0x79, 0x3d, 0x69, 0xd8, 0x47, 0x7d, 0xe4,
];

/// The EFI System Partition type GUID (C12A7328-F81F-11D2-BA4B-00A0C93EC93B).
pub const EFI_GUID: [u8; 16] = [
    0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9, 0x3b,
];

// Partition entry field offsets within the 128-byte entry.
const ENT_TYPE_GUID: core::ops::Range<usize> = 0..16;
const ENT_UNIQUE_GUID: core::ops::Range<usize> = 16..32;
const ENT_STARTING_LBA: core::ops::Range<usize> = 32..40;
const ENT_ENDING_LBA: core::ops::Range<usize> = 40..48;
const ENT_ATTRIBUTES: core::ops::Range<usize> = 48..56;
const ENT_NAME_START: usize = 56;
const ENT_NAME_MAX_BYTES: usize = 72;

/// A GPT partition entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
    /// GPT partition type GUID.
    pub type_guid: [u8; 16],
    /// Unique partition GUID.
    pub unique_guid: [u8; 16],
    /// First LBA of the partition.
    pub starting_lba: u64,
    /// Last LBA of the partition (inclusive).
    pub ending_lba: u64,
    /// Partition attributes bitfield.
    pub attributes: u64,
    /// Partition name.
    pub name: String,
}

impl Partition {
    /// Serializes this entry into 128 bytes.
    #[must_use]
    pub(crate) fn encode(&self) -> [u8; 128] {
        let mut bytes = [0_u8; 128];
        put(&mut bytes, ENT_TYPE_GUID, &self.type_guid);
        put(&mut bytes, ENT_UNIQUE_GUID, &self.unique_guid);
        put(
            &mut bytes,
            ENT_STARTING_LBA,
            &self.starting_lba.to_le_bytes(),
        );
        put(&mut bytes, ENT_ENDING_LBA, &self.ending_lba.to_le_bytes());
        put(&mut bytes, ENT_ATTRIBUTES, &self.attributes.to_le_bytes());

        let mut name_bytes = [0_u8; ENT_NAME_MAX_BYTES];
        for (dst, unit) in name_bytes.chunks_exact_mut(2).zip(self.name.encode_utf16()) {
            dst.copy_from_slice(&unit.to_le_bytes());
        }
        put(
            &mut bytes,
            ENT_NAME_START..ENT_NAME_START.saturating_add(ENT_NAME_MAX_BYTES),
            &name_bytes,
        );

        bytes
    }

    /// Parses a 128-byte entry, returning `None` for unused (zeroed) slots.
    pub(crate) fn decode(bytes: &[u8; 128]) -> Option<Self> {
        let type_guid: [u8; 16] = slice(bytes, ENT_TYPE_GUID)?.try_into().ok()?;
        if type_guid == [0; 16] {
            return None;
        }
        let unique_guid: [u8; 16] = slice(bytes, ENT_UNIQUE_GUID)?.try_into().ok()?;
        let name = decode_name(bytes)?;

        Some(Self {
            type_guid,
            unique_guid,
            starting_lba: le_u64(bytes, ENT_STARTING_LBA)?,
            ending_lba: le_u64(bytes, ENT_ENDING_LBA)?,
            attributes: le_u64(bytes, ENT_ATTRIBUTES)?,
            name,
        })
    }
}

fn decode_name(bytes: &[u8; 128]) -> Option<String> {
    let raw = bytes.get(ENT_NAME_START..ENT_NAME_START.saturating_add(ENT_NAME_MAX_BYTES))?;
    let mut units = Vec::new();
    for chunk in raw.chunks_exact(2) {
        let unit: [u8; 2] = chunk.try_into().ok()?;
        units.push(u16::from_le_bytes(unit));
    }
    let end = units
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(units.len());

    Some(String::from_utf16_lossy(units.get(..end)?))
}

fn slice(bytes: &[u8; 128], range: core::ops::Range<usize>) -> Option<&[u8]> {
    bytes.get(range)
}

fn le_u64(bytes: &[u8; 128], range: core::ops::Range<usize>) -> Option<u64> {
    let value: [u8; 8] = slice(bytes, range)?.try_into().ok()?;

    Some(u64::from_le_bytes(value))
}

fn put(bytes: &mut [u8; 128], range: core::ops::Range<usize>, data: &[u8]) {
    if let Some(dst) = bytes.get_mut(range) {
        dst.copy_from_slice(data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn efi_guid_matches_uefi_spec_value() {
        assert_eq!(
            EFI_GUID,
            [
                0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e,
                0xc9, 0x3b,
            ]
        );
    }

    #[test]
    fn encode_then_decode_round_trips() {
        // ARRANGE
        let partition = Partition {
            type_guid: EFI_GUID,
            unique_guid: [0xAB; 16],
            starting_lba: 2048,
            ending_lba: 4095,
            attributes: 0x42,
            name: "EFI".to_owned(),
        };

        // ACT
        let bytes = partition.encode();
        let decoded = Partition::decode(&bytes).expect("entry must decode");

        // ASSERT
        assert_eq!(decoded, partition);
    }

    #[test]
    fn decode_returns_none_for_zeroed_entry() {
        // ARRANGE
        let bytes = [0_u8; 128];

        // ACT
        let decoded = Partition::decode(&bytes);

        // ASSERT
        assert!(decoded.is_none());
    }

    #[test]
    fn encode_truncates_long_names_to_72_bytes() {
        // ARRANGE
        let partition = Partition {
            type_guid: EFI_GUID,
            unique_guid: [0xAB; 16],
            starting_lba: 2048,
            ending_lba: 4095,
            attributes: 0,
            name: "A".repeat(64),
        };

        // ACT
        let bytes = partition.encode();
        let decoded = Partition::decode(&bytes).expect("entry must decode");

        // ASSERT
        assert_eq!(decoded.name.chars().count(), 36);
        assert!(decoded.name.starts_with("AAAAAAAAAA"));
    }

    #[test]
    fn decode_trims_nul_padding_from_name() {
        // ARRANGE
        let mut partition = Partition {
            type_guid: LINUX_FS_GUID,
            unique_guid: [0xBC; 16],
            starting_lba: 4096,
            ending_lba: 8191,
            attributes: 0,
            name: "DATA".to_owned(),
        };
        let bytes = partition.encode();

        // ACT
        partition.name = "DATA".to_owned();
        let decoded = Partition::decode(&bytes).expect("entry must decode");

        // ASSERT
        assert_eq!(decoded.name, "DATA");
        assert_eq!(decoded.type_guid, LINUX_FS_GUID);
    }
}
