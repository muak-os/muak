//! TPM2 device I/O via /dev/tpmrm0.

use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use crate::errors::{Error, Result};
use crate::types::{ResponseReader, TPM2_ST_NO_SESSIONS, TPM2_ST_SESSIONS};

const TPM_DEVICE: &str = "/dev/tpmrm0";
const MAX_RESPONSE_SIZE: usize = 4096;
const RESPONSE_HEADER_SIZE: usize = 10;

/// Formats a byte slice as grouped hex, 16 bytes per line.
fn hex_dump(label: &str, data: &[u8]) {
    eprintln!("{} ({} bytes):", label, data.len());
    for (i, chunk) in data.chunks(16).enumerate() {
        let hex: Vec<String> = chunk.iter().map(|b| format!("{:02x}", b)).collect();
        eprintln!("  {:04x}  {}", i * 16, hex.join(" "));
    }
}

/// An open handle to the TPM resource manager.
pub struct Device {
    file: File,
}

impl Device {
    /// Opens /dev/tpmrm0.
    pub fn open() -> Result<Self> {
        if !Path::new(TPM_DEVICE).exists() {
            return Err(Error::DeviceNotFound(TPM_DEVICE.to_string()));
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(TPM_DEVICE)?;
        Ok(Self { file })
    }

    /// Sends a command to the TPM and returns the full response buffer.
    pub fn transact(&mut self, command: &[u8]) -> Result<Vec<u8>> {
        if std::env::var("TPM2_DEBUG").is_ok() {
            hex_dump("TPM2 CMD", command);
        }

        self.file.write_all(command)?;
        self.file.flush()?;

        let mut response = vec![0u8; MAX_RESPONSE_SIZE];
        let n = self.file.read(&mut response)?;
        response.truncate(n);

        if std::env::var("TPM2_DEBUG").is_ok() {
            hex_dump("TPM2 RSP", &response);
        }

        validate_response(&response)?;

        Ok(response)
    }
}

/// Validates response header and checks for TPM errors.
fn validate_response(response: &[u8]) -> Result<()> {
    if response.len() < RESPONSE_HEADER_SIZE {
        return Err(Error::ResponseTooShort {
            expected: RESPONSE_HEADER_SIZE,
            actual: response.len(),
        });
    }

    let mut reader = ResponseReader::new(response);
    let tag = reader.read_u16()?;

    if tag != TPM2_ST_NO_SESSIONS && tag != TPM2_ST_SESSIONS {
        return Err(Error::BadResponseTag);
    }

    let _size = reader.read_u32()?;
    let rc = reader.read_u32()?;

    if rc != 0 {
        return Err(Error::TpmError(rc));
    }

    Ok(())
}

/// Returns true if the TPM2 resource manager device exists.
pub fn is_available() -> bool {
    Path::new(TPM_DEVICE).exists()
}
