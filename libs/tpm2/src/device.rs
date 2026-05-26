//! TPM2 device I/O via `/dev/tpmrm0`.

use std::io::{Read, Write};
use std::path::Path;

use crate::error::{Result, Tpm2Error};
use crate::response::ResponseReader;

const TPM_DEVICE: &str = "/dev/tpmrm0";
const MAX_RESPONSE_SIZE: usize = 4096;
const RESPONSE_HEADER_SIZE: usize = 10;
const TPM2_ST_NO_SESSIONS: u16 = 0x8001;
const TPM2_ST_SESSIONS: u16 = 0x8002;

/// Returns true if the TPM2 resource manager device exists.
#[must_use]
pub fn is_available(path: Option<&Path>) -> bool {
    path.unwrap_or_else(|| Path::new(TPM_DEVICE)).exists()
}

/// An open handle to the TPM resource manager.
pub(crate) type Device = std::fs::File;

impl TpmDevice for Device {}

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
        Ok(file) => Ok(file),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Err(Tpm2Error::DeviceNotFound)
        }
        Err(error) => Err(Tpm2Error::Io(error)),
    }
}

/// A trait for sending commands to a TPM device and receiving responses.
pub(crate) trait TpmDevice: Read + Write {
    fn transact(&mut self, command: &[u8]) -> Result<Vec<u8>> {
        self.write_all(command)?;
        self.flush()?;

        let mut response = vec![0_u8; MAX_RESPONSE_SIZE];
        let read_len = self.read(&mut response)?;
        response.truncate(read_len);

        validate_response(&response)?;

        Ok(response)
    }
}

/// Validates response header and checks for TPM errors.
fn validate_response(response: &[u8]) -> Result<()> {
    if response.len() < RESPONSE_HEADER_SIZE {
        return Err(Tpm2Error::ResponseTooShort {
            expected: RESPONSE_HEADER_SIZE,
            actual: response.len(),
        });
    }

    let mut reader = ResponseReader::new(response);
    let tag = reader.read_u16()?;

    if tag != TPM2_ST_NO_SESSIONS && tag != TPM2_ST_SESSIONS {
        return Err(Tpm2Error::BadResponseTag);
    }

    let _size = reader.read_u32()?;
    let rc = reader.read_u32()?;

    if rc != 0 {
        return Err(Tpm2Error::TpmError(rc));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::io::{Error as IoError, ErrorKind, Result as IoResult};
    use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[derive(Default)]
    struct MockIo {
        response: VecDeque<u8>,
        written: Vec<u8>,
        fail_write: bool,
    }

    impl MockIo {
        fn new(response: Vec<u8>) -> Self {
            Self {
                response: response.into(),
                written: Vec::new(),
                fail_write: false,
            }
        }
    }

    impl Read for MockIo {
        fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
            let read_len = buf.len().min(self.response.len());
            for byte in buf.iter_mut().take(read_len) {
                let value = match self.response.pop_front() {
                    Some(value) => value,
                    None => panic!("response should contain enough bytes"),
                };
                *byte = value;
            }
            Ok(read_len)
        }
    }

    impl Write for MockIo {
        fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
            if self.fail_write {
                return Err(IoError::new(ErrorKind::BrokenPipe, "write failed"));
            }
            self.written.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> IoResult<()> {
            Ok(())
        }
    }

    impl TpmDevice for MockIo {}

    fn response(tag: u16, rc: u32, body: &[u8]) -> Vec<u8> {
        let size = 10_usize + body.len();
        let mut out = Vec::with_capacity(size);
        out.extend_from_slice(&tag.to_be_bytes());
        let size = u32::try_from(size).ok().unwrap_or(0);
        out.extend_from_slice(&size.to_be_bytes());
        out.extend_from_slice(&rc.to_be_bytes());
        out.extend_from_slice(body);

        out
    }

    #[test]
    fn validate_response_accepts_success_tags() {
        // ARRANGE
        let no_sessions = response(TPM2_ST_NO_SESSIONS, 0, &[]);
        let sessions = response(TPM2_ST_SESSIONS, 0, &[]);

        // ACT
        let no_sessions_result = validate_response(&no_sessions);
        let sessions_result = validate_response(&sessions);

        // ASSERT
        assert!(
            no_sessions_result.is_ok(),
            "no-sessions response should validate"
        );
        assert!(sessions_result.is_ok(), "sessions response should validate");
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        let unique = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(duration) => duration.as_nanos(),
            Err(_) => 0,
        };
        std::env::temp_dir().join(format!("tpm2-{name}-{unique}"))
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
    fn device_alias_supports_read_write_and_flush() {
        // ARRANGE
        let path = temp_path("io");
        let file_result = std::fs::OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path);
        assert!(file_result.is_ok(), "temporary file should be created");
        let mut device = match open(Some(&path)) {
            Ok(device) => device,
            Err(_) => panic!("file should open"),
        };

        // ACT
        let write_result = device.write_all(&[1, 2, 3]);
        let flush_result = device.flush();
        let seek_result = device.seek(SeekFrom::Start(0));
        let mut buffer = [0_u8; 3];
        let read_result = device.read_exact(&mut buffer);

        // ASSERT
        assert!(
            write_result.is_ok(),
            "device should write through file alias"
        );
        assert!(
            flush_result.is_ok(),
            "device should flush through file alias"
        );
        assert!(seek_result.is_ok(), "device should seek for verification");
        assert!(read_result.is_ok(), "device should read through file alias");
        assert_eq!(buffer, [1, 2, 3], "written bytes should roundtrip");
        assert!(
            std::fs::remove_file(&path).is_ok(),
            "temporary file should be removed"
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

    #[test]
    fn validate_response_rejects_bad_headers() {
        // ARRANGE
        let short = [0_u8; 9];
        let bad_tag = response(0xFFFF, 0, &[]);
        let tpm_error = response(TPM2_ST_NO_SESSIONS, 0x101, &[]);

        // ACT
        let short_result = validate_response(&short);
        let bad_tag_result = validate_response(&bad_tag);
        let tpm_error_result = validate_response(&tpm_error);

        // ASSERT
        assert!(short_result.is_err(), "short response should fail");
        assert!(bad_tag_result.is_err(), "bad tag should fail");
        assert!(tpm_error_result.is_err(), "TPM error code should fail");
    }

    #[test]
    fn transact_writes_command_and_returns_response() {
        // ARRANGE
        let expected_response = response(TPM2_ST_NO_SESSIONS, 0, &[1, 2, 3]);
        let mut io = MockIo::new(expected_response.clone());
        let command = [0xAA, 0xBB];

        // ACT
        let actual_response = TpmDevice::transact(&mut io, &command);

        // ASSERT
        assert!(actual_response.is_ok(), "transaction should succeed");
        assert_eq!(io.written, command, "command should be written to device");
        assert_eq!(
            actual_response.ok(),
            Some(expected_response),
            "response should be returned"
        );
    }

    #[test]
    fn device_transact_and_trait_execute_use_default_transaction() {
        // ARRANGE
        let expected_response = response(TPM2_ST_NO_SESSIONS, 0, &[1]);
        let mut trait_io = MockIo::new(expected_response.clone());

        // ACT
        let through_trait = TpmDevice::transact(&mut trait_io, &[0xBB]);

        // ASSERT
        assert_eq!(
            through_trait.ok(),
            Some(expected_response),
            "trait transaction should work"
        );
    }

    #[test]
    fn transact_propagates_io_errors() {
        // ARRANGE
        let mut io = MockIo::new(Vec::new());
        io.fail_write = true;

        // ACT
        let result = TpmDevice::transact(&mut io, &[0xAA]);

        // ASSERT
        assert!(result.is_err(), "write failure should be propagated");
    }
}
