//! Default TPM2 transport backed by `/dev/tpmrm0`.

use std::io::{Read, Write as _};
use std::path::Path;

use zeroize::Zeroizing;

use crate::blob::Sealed;
use crate::error::{Result, Tpm2Error};
use crate::operations::{self, SealResult};
use crate::pcr::Digest;
use crate::response::{RESPONSE_HEADER_SIZE, ResponseReader};
use crate::transport::Transport;

const TPM_DEVICE: &str = "/dev/tpmrm0";
const MAX_RESPONSE_SIZE: usize = 4096;
const NONCE_SIZE: usize = 32;

/// An open handle to the TPM resource manager.
pub(crate) struct Device {
    file: std::fs::File,
    response: [u8; MAX_RESPONSE_SIZE],
}

impl Transport for Device {
    fn transact(&mut self, request: &[u8]) -> Result<&[u8]> {
        self.file.write_all(request)?;
        self.file.flush()?;

        read_response(&mut self.file, &mut self.response)
    }
}

/// Returns true if the TPM2 resource manager device exists.
#[must_use = "callers must branch on whether a TPM is present"]
pub fn is_available(path: Option<&Path>) -> bool {
    path.unwrap_or_else(|| Path::new(TPM_DEVICE)).exists()
}

/// Seals data to the default TPM2 device.
///
/// # Errors
///
/// Returns an error if the device cannot be opened or the seal operation fails.
pub fn seal(data: &[u8], expected_pcr: &Digest) -> Result<SealResult> {
    let mut device = open(None)?;

    operations::seal(&mut device, data, expected_pcr)
}

/// Unseals data using the default TPM2 device.
///
/// # Errors
///
/// Returns an error if the device cannot be opened, randomness fails, or the unseal operation fails.
pub fn unseal(blob: &Sealed) -> Result<Zeroizing<Vec<u8>>> {
    let mut device = open(None)?;
    let mut nonce = [0_u8; NONCE_SIZE];
    getrandom::fill(&mut nonce).map_err(|_rng_error| Tpm2Error::RngFailed)?;

    operations::unseal(&mut device, blob, &nonce)
}

/// Opens the provided TPM device path, defaulting to `/dev/tpmrm0`.
///
/// # Errors
///
/// Returns an error if the TPM resource manager cannot be opened.
pub(crate) fn open(path: Option<&Path>) -> Result<Device> {
    let device_path = path.unwrap_or_else(|| Path::new(TPM_DEVICE));
    let file_result = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(device_path);

    match file_result {
        Ok(file) => Ok(Device {
            file,
            response: [0_u8; MAX_RESPONSE_SIZE],
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Err(Tpm2Error::DeviceNotFound)
        }
        Err(error) => Err(Tpm2Error::Io(error)),
    }
}

fn read_response<'b>(reader: &mut impl Read, buffer: &'b mut [u8]) -> Result<&'b [u8]> {
    let mut header = [0_u8; RESPONSE_HEADER_SIZE];
    reader.read_exact(&mut header)?;

    let size = response_size(&header)?;
    if size > buffer.len() {
        return Err(Tpm2Error::ResponseTooLarge {
            max: buffer.len(),
            actual: size,
        });
    }

    let (response, _) = buffer.split_at_mut(size);
    let (head, body) = response.split_at_mut(RESPONSE_HEADER_SIZE);
    head.copy_from_slice(&header);
    reader.read_exact(body)?;

    Ok(response)
}

fn response_size(header: &[u8]) -> Result<usize> {
    let mut reader = ResponseReader::new(header);
    let _tag = reader.read_u16()?;
    let size = usize::try_from(reader.read_u32()?).unwrap_or(usize::MAX);

    if size < RESPONSE_HEADER_SIZE {
        return Err(Tpm2Error::ResponseTooShort {
            expected: RESPONSE_HEADER_SIZE,
            actual: size,
        });
    }

    Ok(size)
}

#[cfg(test)]
mod tests {
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::tags::TPM2_ST_NO_SESSIONS;

    fn response(tag: u16, rc: u32, body: &[u8]) -> Vec<u8> {
        let size = 10_usize
            .checked_add(body.len())
            .expect("response size should fit usize");
        let mut out = Vec::with_capacity(size);
        out.extend_from_slice(&tag.to_be_bytes());
        let size = u32::try_from(size).ok().unwrap_or(0);
        out.extend_from_slice(&size.to_be_bytes());
        out.extend_from_slice(&rc.to_be_bytes());
        out.extend_from_slice(body);

        out
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        let unique = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(duration) => duration.as_nanos(),
            Err(_) => 0,
        };
        std::env::temp_dir().join(format!("tpm2-{name}-{unique}"))
    }

    #[test]
    fn read_response_rejects_short_header() {
        // ARRANGE
        let data = [0x80, 0x01, 0x00, 0x00];
        let mut reader: &[u8] = &data;
        let mut buffer = [0_u8; MAX_RESPONSE_SIZE];

        // ACT
        let result = read_response(&mut reader, &mut buffer);

        // ASSERT
        assert!(result.is_err(), "short header should be rejected");
    }

    #[test]
    fn read_response_rejects_short_declared_size() {
        // ARRANGE
        let data = [0x80, 0x01, 0x00, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00, 0x00];
        let mut reader: &[u8] = &data;
        let mut buffer = [0_u8; MAX_RESPONSE_SIZE];

        // ACT
        let result = read_response(&mut reader, &mut buffer);

        // ASSERT
        assert!(
            result.is_err(),
            "declared size below the header should be rejected"
        );
    }

    #[test]
    fn read_response_rejects_oversized_response() {
        // ARRANGE
        let mut data = [0_u8; RESPONSE_HEADER_SIZE];
        data.get_mut(..2)
            .expect("tag range should exist")
            .copy_from_slice(&TPM2_ST_NO_SESSIONS.to_be_bytes());
        data.get_mut(2..6)
            .expect("size range should exist")
            .copy_from_slice(&100_u32.to_be_bytes());
        let mut reader: &[u8] = &data;
        let mut buffer = [0_u8; 16];

        // ACT
        let result = read_response(&mut reader, &mut buffer);

        // ASSERT
        assert!(
            result.is_err(),
            "response larger than the buffer should be rejected"
        );
    }

    #[test]
    fn read_response_reads_header_and_body() {
        // ARRANGE
        let data = response(TPM2_ST_NO_SESSIONS, 0, &[1, 2, 3]);
        let mut reader: &[u8] = &data;
        let mut buffer = [0_u8; MAX_RESPONSE_SIZE];

        // ACT
        let result = read_response(&mut reader, &mut buffer);

        // ASSERT
        assert_eq!(
            result.ok(),
            Some(data.as_slice()),
            "whole response should be returned"
        );
        assert_eq!(
            buffer.get(..data.len()),
            Some(data.as_slice()),
            "response bytes should be read"
        );
    }

    #[test]
    fn read_response_rejects_truncated_body() {
        // ARRANGE
        let data = response(TPM2_ST_NO_SESSIONS, 0, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        let truncated = data
            .get(..13)
            .expect("truncated response range should exist");
        let mut reader: &[u8] = truncated;
        let mut buffer = [0_u8; MAX_RESPONSE_SIZE];

        // ACT
        let result = read_response(&mut reader, &mut buffer);

        // ASSERT
        assert!(
            matches!(result, Err(Tpm2Error::Io(_))),
            "truncated body should surface as an I/O error"
        );
    }

    #[test]
    fn transact_writes_request_and_returns_response() {
        // ARRANGE
        let (device_stream, mut peer) =
            UnixStream::pair().expect("unix stream pair should be created");
        let file = std::fs::File::from(OwnedFd::from(device_stream));
        let mut device = Device {
            file,
            response: [0_u8; MAX_RESPONSE_SIZE],
        };
        let request = [
            0x80_u8, 0x01, 0x00, 0x00, 0x00, 0x0A, 0x00, 0x00, 0x01, 0x7B,
        ];
        let expected = response(TPM2_ST_NO_SESSIONS, 0, &[0xAA, 0xBB, 0xCC]);
        peer.write_all(
            expected
                .get(..RESPONSE_HEADER_SIZE)
                .expect("header should exist"),
        )
        .expect("response header should reach the peer");
        peer.write_all(
            expected
                .get(RESPONSE_HEADER_SIZE..)
                .expect("body should exist"),
        )
        .expect("response body should reach the peer");

        // ACT
        let result = Transport::transact(&mut device, &request);
        let mut received = [0_u8; 10];
        peer.read_exact(&mut received)
            .expect("request should be readable from the peer");

        // ASSERT
        assert_eq!(
            result.ok(),
            Some(expected.as_slice()),
            "transact should return the whole response"
        );
        assert_eq!(received, request, "peer should receive exactly the request");
    }

    #[test]
    fn transact_propagates_io_errors() {
        // ARRANGE
        let path = temp_path("transact-readonly");
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .expect("temporary file should be created");
        let file = std::fs::OpenOptions::new()
            .read(true)
            .open(&path)
            .expect("temporary file should be reopened read-only");
        let mut device = Device {
            file,
            response: [0_u8; MAX_RESPONSE_SIZE],
        };

        // ACT
        let result = Transport::transact(&mut device, &[0x80, 0x01]);

        // ASSERT
        assert!(
            matches!(result, Err(Tpm2Error::Io(_))),
            "read-only file should surface a write failure as an I/O error"
        );
        assert!(
            std::fs::remove_file(&path).is_ok(),
            "temporary file should be removed"
        );
    }

    #[test]
    fn device_open_reports_missing_device() {
        // ACT
        let result = open(None);

        // ASSERT
        assert!(
            result.is_err(),
            "test environment should not expose a TPM device"
        );
    }

    #[test]
    fn device_open_reports_missing_custom_device() {
        // ARRANGE
        let missing_path = Path::new("/definitely/not/a/tpm-device");

        // ACT
        let result = open(Some(missing_path));

        // ASSERT
        assert!(
            result.is_err(),
            "missing custom TPM device should be reported"
        );
    }

    #[test]
    fn open_uses_custom_file_path() {
        // ARRANGE
        let path = temp_path("open");
        let file_result = std::fs::OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path);
        assert!(file_result.is_ok(), "temporary file should be created");

        // ACT
        let opened = open(Some(&path));

        // ASSERT
        assert!(opened.is_ok(), "custom file path should open successfully");
        assert!(
            std::fs::remove_file(&path).is_ok(),
            "temporary file should be removed"
        );
    }

    #[test]
    fn seal_and_unseal_propagate_open_failures() {
        // ARRANGE
        let blob = Sealed::try_new(Vec::new(), Vec::new()).expect("empty blob should be valid");

        // ACT
        let seal_result = seal(&[], &[0x11; 32]);
        let unseal_result = unseal(&blob);

        // ASSERT
        assert!(
            seal_result.is_err(),
            "seal wrapper should propagate open failure"
        );
        assert!(
            unseal_result.is_err(),
            "unseal wrapper should propagate open failure"
        );
    }

    #[test]
    fn availability_matches_device_path() {
        // ACT
        let available = is_available(None);

        // ASSERT
        assert_eq!(
            available,
            Path::new(TPM_DEVICE).exists(),
            "availability should check TPM path"
        );
    }

    #[test]
    fn availability_checks_custom_path() {
        // ARRANGE
        let missing_path = Path::new("/definitely/not/a/tpm-device");

        // ACT
        let available = is_available(Some(missing_path));

        // ASSERT
        assert!(!available, "missing custom path should report unavailable");
    }
}
