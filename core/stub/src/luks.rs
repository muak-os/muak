//! LUKS key injection from ESP file for systems without TPM2 support.

use anyhow::{Context as _, Result, anyhow};
use base64ct::{Base64Unpadded, Encoding as _};
use uefi::Handle;
use uefi::boot::open_protocol_exclusive;
use uefi::proto::media::file::{File, FileAttribute, FileMode};
use uefi::proto::media::fs::SimpleFileSystem;

use crate::pe::loader::cmdline::strip_trailing_terminators;

const LUKS_KEY_PREFIX: &[u8] = b" luks.key=";
const MAX_LUKS_KEY_SIZE: usize = 65536;

/// Reads the LUKS key from the ESP `luks` file and injects it into the kernel cmdline.
pub fn try_inject(device_handle: Handle, cmdline: Option<&[u8]>) -> Result<Option<Vec<u8>>> {
    let luks_data = match read_key(device_handle)? {
        Some(data) => data,
        None => return Ok(None),
    };

    let base_cmd = strip_trailing_terminators(cmdline.unwrap_or(&[]));
    let encoded_len = Base64Unpadded::encoded_len(&luks_data);

    let total_len = base_cmd
        .len()
        .checked_add(LUKS_KEY_PREFIX.len())
        .and_then(|len| len.checked_add(encoded_len))
        .context("combined command line length overflow")?;
    let mut combined = Vec::with_capacity(total_len);
    combined.extend_from_slice(base_cmd);
    combined.extend_from_slice(LUKS_KEY_PREFIX);

    let start = combined.len();
    combined.resize(total_len, 0);
    let dst = combined
        .get_mut(start..)
        .context("LUKS key destination range unavailable")?;
    Base64Unpadded::encode(&luks_data, dst).context("Failed to encode LUKS key")?;

    Ok(Some(combined))
}

fn read_key(device_handle: Handle) -> Result<Option<Vec<u8>>> {
    let mut fs = match open_protocol_exclusive::<SimpleFileSystem>(device_handle) {
        Ok(fs) => fs,
        Err(_) => return Ok(None),
    };
    let mut dir = match fs.open_volume() {
        Ok(dir) => dir,
        Err(_) => return Ok(None),
    };

    let filename = uefi::cstr16!("luks");
    let file = match dir.open(filename, FileMode::Read, FileAttribute::empty()) {
        Ok(file) => file,
        Err(err) => {
            if err.status() == uefi::Status::NOT_FOUND {
                return Ok(None);
            }
            return Err(anyhow!("Failed to open luks file: {err}"));
        }
    };
    let mut regular_file = file
        .into_regular_file()
        .ok_or_else(|| anyhow!("luks file is not a regular file"))?;

    let mut buf = vec![0u8; MAX_LUKS_KEY_SIZE];
    let bytes_read = regular_file
        .read(&mut buf)
        .context("Failed to read luks file")?;
    if bytes_read == 0 {
        return Ok(None);
    }
    buf.truncate(bytes_read);

    Ok(Some(buf))
}
