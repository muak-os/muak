//! EFI System Partition type GUID matching.

use crate::EFI_GUID;

/// Returns `true` when the partition type GUID is the EFI System Partition type.
#[must_use]
pub fn is_esp(type_guid: &str) -> bool {
    type_guid.eq_ignore_ascii_case(&canonical_text())
}

fn canonical_text() -> String {
    let guid = &EFI_GUID;
    format!(
        "{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        guid[3],
        guid[2],
        guid[1],
        guid[0],
        guid[5],
        guid[4],
        guid[7],
        guid[6],
        guid[8],
        guid[9],
        guid[10],
        guid[11],
        guid[12],
        guid[13],
        guid[14],
        guid[15],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_esp_guid() {
        // ARRANGE / ACT
        let result = is_esp("C12A7328-F81F-11D2-BA4B-00A0C93EC93B");

        // ASSERT
        assert!(result);
    }

    #[test]
    fn accepts_lowercase_esp_guid() {
        // ARRANGE / ACT
        let result = is_esp("c12a7328-f81f-11d2-ba4b-00a0c93ec93b");

        // ASSERT
        assert!(result);
    }

    #[test]
    fn rejects_non_esp_guid() {
        // ARRANGE
        let guid = "11111111-1111-1111-1111-111111111111";

        // ACT
        let result = is_esp(guid);

        // ASSERT
        assert!(!result);
    }
}
