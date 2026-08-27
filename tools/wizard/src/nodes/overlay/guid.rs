//! Textual GUID parsing and EFI System Partition type validation.

use crate::error::{Result, WizardError};

/// Rejects a partition type GUID unless it is the EFI System Partition type.
///
/// # Errors
///
/// Returns an error when the GUID is malformed or is not the ESP type GUID.
pub(crate) fn assert_esp(type_guid: &str) -> Result<()> {
    if parse(type_guid)? == esp::EFI_GUID {
        Ok(())
    } else {
        Err(WizardError::BuildError(format!(
            "unsupported partition type GUID {type_guid}",
        )))
    }
}

macro_rules! hex_field {
    ($ty:ty, $field:expr) => {
        <$ty>::from_str_radix($field, 16).map_err(|err| {
            WizardError::BuildError(format!("malformed GUID field: {} ({err})", $field))
        })?
    };
}

/// Parses a textual GUID (`C12A7328-F81F-11D2-BA4B-00A0C93EC93B`) into bytes.
fn parse(text: &str) -> Result<[u8; 16]> {
    let fields: Vec<&str> = text.split('-').collect();
    let &[low, mid, high, clock, node] = fields.as_slice() else {
        return Err(WizardError::BuildError(format!("malformed GUID: {text}")));
    };
    let time_low = hex_field!(u32, low);
    let time_mid = hex_field!(u16, mid);
    let time_hi = hex_field!(u16, high);
    let clock = hex_field!(u16, clock);
    let node_value = hex_field!(u64, node);
    let node = node_value.to_be_bytes();
    let mut bytes = [0_u8; 16];
    bytes[0..4].copy_from_slice(&time_low.to_le_bytes());
    bytes[4..6].copy_from_slice(&time_mid.to_le_bytes());
    bytes[6..8].copy_from_slice(&time_hi.to_le_bytes());
    bytes[8..10].copy_from_slice(&clock.to_be_bytes());
    bytes[10..16].copy_from_slice(&node[2..8]);

    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_esp_guid() {
        // ARRANGE / ACT
        let result = assert_esp("C12A7328-F81F-11D2-BA4B-00A0C93EC93B");

        // ASSERT
        result.unwrap();
    }

    #[test]
    fn accepts_lowercase_esp_guid() {
        // ARRANGE / ACT
        let result = assert_esp("c12a7328-f81f-11d2-ba4b-00a0c93ec93b");

        // ASSERT
        result.unwrap();
    }

    #[test]
    fn rejects_non_esp_guid() {
        // ARRANGE
        let guid = "11111111-1111-1111-1111-111111111111";

        // ACT
        let result = assert_esp(guid);

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn rejects_malformed_guid() {
        // ARRANGE
        let guid = "not-a-guid";

        // ACT
        let result = assert_esp(guid);

        // ASSERT
        result.unwrap_err();
    }
}
