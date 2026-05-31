//! Functions for building the DM table load buffer for a LUKS2 encrypted volume.

use core::mem::size_of;

use zeroize::Zeroize as _;

use super::abi::{
    DM_TABLE_BUF_SIZE, DmIoctl, DmTargetSpec, TARGET_TYPE, copy_prefix, dm_ioctl_bytes,
    dm_target_spec_bytes, usize_to_u32,
};
use super::crypt::CryptParams;
use crate::error::{Luks2Error as Error, Result};

pub(super) fn build_buffer(params: &CryptParams<'_>) -> Result<Vec<u8>> {
    let mut key_hex = hex_encode(params.volume_key);
    let params_string = table_params_string(params, &key_hex);
    key_hex.zeroize();

    let params_bytes = params_string.as_bytes();
    let target_spec_size = size_of::<DmTargetSpec>();
    let header_size = size_of::<DmIoctl>();
    let next_offset = target_spec_size
        .checked_add(params_bytes.len())
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| Error::InvalidField("dm table target size overflow".into()))?;
    let total_size = header_size
        .checked_add(next_offset)
        .ok_or_else(|| Error::InvalidField("dm table buffer size overflow".into()))?;
    let buffer_size = total_size.max(DM_TABLE_BUF_SIZE);

    let mut buffer = vec![0_u8; buffer_size];

    let mut header = DmIoctl::with_name(params.name, usize_to_u32(buffer_size)?)?;
    header.target_count = 1;
    write_bytes(&mut buffer, 0, dm_ioctl_bytes(&header))?;

    let mut target = DmTargetSpec {
        sector_start: 0,
        length: params.size_sectors,
        status: 0,
        next: usize_to_u32(next_offset)?,
        target_type: [0_u8; 16],
    };
    copy_prefix(&mut target.target_type, TARGET_TYPE);
    write_bytes(&mut buffer, header_size, dm_target_spec_bytes(&target))?;

    let params_offset = header_size
        .checked_add(target_spec_size)
        .ok_or_else(|| Error::InvalidField("dm params offset overflow".into()))?;
    write_bytes(&mut buffer, params_offset, params_bytes)?;
    write_byte(
        &mut buffer,
        params_offset
            .checked_add(params_bytes.len())
            .ok_or_else(|| Error::InvalidField("dm params terminator overflow".into()))?,
        0,
    )?;

    Ok(buffer)
}

fn table_params_string(params: &CryptParams<'_>, key_hex: &str) -> String {
    if params.sector_size == 512 {
        format!(
            "{} {} 0 {} {} 2 allow_discards no_read_workqueue",
            params.cipher, key_hex, params.backing_device, params.offset_sectors
        )
    } else {
        format!(
            "{} {} 0 {} {} 3 allow_discards sector_size:{} no_read_workqueue",
            params.cipher,
            key_hex,
            params.backing_device,
            params.offset_sectors,
            params.sector_size
        )
    }
}

fn hex_encode(data: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut hex = String::with_capacity(data.len().saturating_mul(2));
    for &byte in data {
        let high = HEX.get(usize::from(byte >> 4)).copied().unwrap_or(b'0');
        let low = HEX.get(usize::from(byte & 0x0f)).copied().unwrap_or(b'0');
        hex.push(char::from(high));
        hex.push(char::from(low));
    }

    hex
}

fn write_bytes(buffer: &mut [u8], offset: usize, src: &[u8]) -> Result<()> {
    let end = offset
        .checked_add(src.len())
        .ok_or_else(|| Error::InvalidField("buffer write overflow".into()))?;
    let dst = buffer
        .get_mut(offset..end)
        .ok_or_else(|| Error::InvalidField("buffer write out of bounds".into()))?;
    dst.copy_from_slice(src);

    Ok(())
}

fn write_byte(buffer: &mut [u8], offset: usize, value: u8) -> Result<()> {
    let dst = buffer
        .get_mut(offset)
        .ok_or_else(|| Error::InvalidField("buffer byte write out of bounds".into()))?;
    *dst = value;

    Ok(())
}

#[cfg(test)]
mod tests {
    use core::mem::size_of;

    use super::*;

    fn test_params(sector_size: u32, volume_key: &[u8]) -> CryptParams<'_> {
        CryptParams {
            name: "crypt-test",
            dm_uuid: "CRYPT-LUKS2-deadbeef",
            backing_device: "/dev/loop0",
            volume_key,
            cipher: "aes-xts-plain64",
            offset_sectors: 1024,
            size_sectors: 2048,
            sector_size,
        }
    }

    #[test]
    fn hex_encode_outputs_lowercase_hex() {
        // ARRANGE
        let data = [0x00, 0x0f, 0xa5, 0xff];

        // ACT
        let encoded = hex_encode(&data);

        // ASSERT
        assert_eq!(encoded, "000fa5ff");
    }

    #[test]
    fn table_params_string_omits_sector_size_for_512_byte_sectors() {
        // ARRANGE
        let params = test_params(512, &[0x42; 4]);

        // ACT
        let params_string = table_params_string(&params, "abcd");

        // ASSERT
        assert!(params_string.contains("abcd"));
        assert!(params_string.contains("allow_discards no_read_workqueue"));
        assert!(!params_string.contains("sector_size:"));
    }

    #[test]
    fn table_params_string_includes_sector_size_for_non_default_sectors() {
        // ARRANGE
        let params = test_params(4096, &[0x42; 4]);

        // ACT
        let params_string = table_params_string(&params, "abcd");

        // ASSERT
        assert!(params_string.contains("sector_size:4096"));
        assert!(params_string.contains("3 allow_discards"));
    }

    #[test]
    fn build_table_load_buffer_embeds_header_target_and_params() {
        // ARRANGE
        let volume_key = [0x42_u8; 4];
        let params = test_params(4096, &volume_key);
        let expected_params = table_params_string(&params, &hex_encode(&volume_key));

        // ACT
        let buffer = build_buffer(&params).unwrap();

        // ASSERT
        assert_eq!(buffer.len(), DM_TABLE_BUF_SIZE);

        let header_size = size_of::<DmIoctl>();
        let target_size = size_of::<DmTargetSpec>();
        let params_offset = header_size + target_size;
        let params_end = params_offset + expected_params.len();

        assert_eq!(
            buffer.get(params_offset..params_end).unwrap(),
            expected_params.as_bytes()
        );
        assert_eq!(*buffer.get(params_end).unwrap(), 0);
        assert_eq!(
            buffer.get(header_size + 24..header_size + 29).unwrap(),
            TARGET_TYPE
        );
    }

    #[test]
    fn write_bytes_rejects_out_of_bounds_write() {
        // ARRANGE
        let mut buffer = [0_u8; 2];

        // ACT
        let result = write_bytes(&mut buffer, 1, &[1, 2]);

        // ASSERT
        assert!(
            matches!(result, Err(Error::InvalidField(field)) if field == "buffer write out of bounds")
        );
    }

    #[test]
    fn write_byte_rejects_out_of_bounds_write() {
        // ARRANGE
        let mut buffer = [0_u8; 2];

        // ACT
        let result = write_byte(&mut buffer, 2, 1);

        // ASSERT
        assert!(
            matches!(result, Err(Error::InvalidField(field)) if field == "buffer byte write out of bounds")
        );
    }
}
