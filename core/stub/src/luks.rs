//! LUKS key injection from ESP file for systems without TPM2 support.

use anyhow::{Context as _, Result, anyhow};
use base64ct::{Base64Unpadded, Encoding as _};

use crate::pe::loader::cmdline::strip_trailing_terminators;

const LUKS_KEY_PREFIX: &[u8] = b" luks.key=";

/// Reads the LUKS key from the ESP `luks` file and injects it into the kernel cmdline.
pub fn try_inject(cmdline: Option<&[u8]>) -> Result<Option<Vec<u8>>> {
    let luks_data = match read_key()? {
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

fn read_key() -> Result<Option<Vec<u8>>> {
    match std::fs::read("luks") {
        Ok(data) if !data.is_empty() => Ok(Some(data)),
        Ok(_) => Ok(None),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(anyhow!("Failed to read luks file: {e}")),
    }
}
