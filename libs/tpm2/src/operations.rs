//! High-level TPM2 seal/unseal operations.

use zeroize::Zeroizing;

use crate::blob::SealedBlob;
use crate::commands::{
    self, CreateCommand, CreatePrimaryCommand, EvictControlCommand, FlushContextCommand,
    HandleExistsCommand, LoadCommand, PolicyPcrCommand, StartAuthSessionCommand, UnsealCommand,
};
use crate::device::{self, TpmDevice};
use crate::error::Result;
use crate::handles::{HierarchyHandle, PersistentHandle};
use crate::pcr::{Digest, compute_policy_digest};

const SRK_HANDLE: PersistentHandle = PersistentHandle::new(0x8100_0001);

pub struct SealResult {
    pub blob: SealedBlob,
    pub policy_digest: Digest,
}

/// Seals data to PCR#11 with the given expected PCR value.
///
/// # Errors
///
/// Returns an error if TPM access or command execution fails.
pub fn seal(data: &[u8], expected_pcr: &Digest) -> Result<SealResult> {
    let mut dev = device::open(None)?;
    seal_with_device(&mut dev, data, expected_pcr)
}

fn seal_with_device(
    dev: &mut impl TpmDevice,
    data: &[u8],
    expected_pcr: &Digest,
) -> Result<SealResult> {
    ensure_srk(dev)?;

    let policy_digest = compute_policy_digest(expected_pcr);
    let (pub_data, priv_data) = commands::execute(
        dev,
        &CreateCommand {
            parent_handle: SRK_HANDLE,
            policy_digest: &policy_digest,
            data,
        },
    )?;

    Ok(SealResult {
        blob: SealedBlob::try_new(pub_data, priv_data)?,
        policy_digest,
    })
}

/// Unseals data using current PCR#11 values.
///
/// # Errors
///
/// Returns an error if TPM access, object loading, policy setup, or unsealing fails.
pub fn unseal(blob: &SealedBlob) -> Result<Zeroizing<Vec<u8>>> {
    let mut dev = device::open(None)?;
    unseal_with_device(&mut dev, blob)
}

fn unseal_with_device(dev: &mut impl TpmDevice, blob: &SealedBlob) -> Result<Zeroizing<Vec<u8>>> {
    ensure_srk(dev)?;

    let obj_handle = commands::execute(
        dev,
        &LoadCommand {
            parent_handle: SRK_HANDLE,
            pub_data: blob.public(),
            priv_data: blob.private(),
        },
    )?;
    let session = commands::execute(dev, &StartAuthSessionCommand)?;

    let result = (|| -> Result<Zeroizing<Vec<u8>>> {
        commands::execute(
            dev,
            &PolicyPcrCommand {
                session_handle: session,
                pcr_digest: &[],
            },
        )?;
        commands::execute(
            dev,
            &UnsealCommand {
                object_handle: obj_handle,
                session_handle: session,
            },
        )
    })();

    drop(commands::execute(
        dev,
        &FlushContextCommand { handle: session },
    ));
    drop(commands::execute(
        dev,
        &FlushContextCommand { handle: obj_handle },
    ));

    result
}

/// Ensures the SRK exists at the well-known persistent handle.
fn ensure_srk(dev: &mut impl TpmDevice) -> Result<()> {
    if commands::execute(dev, &HandleExistsCommand { handle: SRK_HANDLE })? {
        return Ok(());
    }

    let (transient_handle, _pub) = commands::execute(dev, &CreatePrimaryCommand)?;

    let result = commands::execute(
        dev,
        &EvictControlCommand {
            auth: HierarchyHandle::OWNER,
            object: transient_handle,
            persistent: SRK_HANDLE,
        },
    );
    drop(commands::execute(
        dev,
        &FlushContextCommand {
            handle: transient_handle,
        },
    ));
    result?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Result as IoResult, Write};

    use super::*;
    use crate::commands::TpmCommand as _;
    const TPM2_ST_NO_SESSIONS: u16 = 0x8001;
    const TPM2_CAP_HANDLES: u32 = 0x0000_0001;

    #[derive(Default)]
    struct MockDevice {
        responses: Vec<Vec<u8>>,
        next_response: usize,
    }

    impl MockDevice {
        fn new(responses: Vec<Vec<u8>>) -> Self {
            Self {
                responses,
                next_response: 0,
            }
        }
    }

    impl Read for MockDevice {
        fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
            let response = self
                .responses
                .get(self.next_response)
                .map_or(&[][..], Vec::as_slice);
            self.next_response = self
                .next_response
                .checked_add(1)
                .expect("response index should fit usize");

            let read_len = response.len();
            buf.get_mut(..read_len)
                .expect("target slice should contain response")
                .copy_from_slice(response);
            Ok(read_len)
        }
    }

    impl Write for MockDevice {
        fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
            Ok(buf.len())
        }

        fn flush(&mut self) -> IoResult<()> {
            Ok(())
        }
    }

    impl TpmDevice for MockDevice {}

    fn body_response(body: &[u8]) -> Vec<u8> {
        let size = 10_usize
            .checked_add(body.len())
            .expect("response size should fit usize");
        let mut out = Vec::with_capacity(size);
        out.extend_from_slice(&TPM2_ST_NO_SESSIONS.to_be_bytes());
        out.extend_from_slice(&u32::try_from(size).ok().unwrap_or(0).to_be_bytes());
        out.extend_from_slice(&0_u32.to_be_bytes());
        out.extend_from_slice(body);

        out
    }

    fn handle_exists_response(found: bool) -> Vec<u8> {
        let mut body = Vec::new();
        body.push(0);
        body.extend_from_slice(&TPM2_CAP_HANDLES.to_be_bytes());
        body.extend_from_slice(&u32::from(found).to_be_bytes());
        if found {
            body.extend_from_slice(&u32::from(SRK_HANDLE).to_be_bytes());
        }

        body_response(&body)
    }

    fn sized(data: &[u8]) -> Vec<u8> {
        let capacity = 2_usize
            .checked_add(data.len())
            .expect("sized buffer capacity should fit usize");
        let mut out = Vec::with_capacity(capacity);
        out.extend_from_slice(&u16::try_from(data.len()).ok().unwrap_or(0).to_be_bytes());
        out.extend_from_slice(data);

        out
    }

    #[test]
    fn ensure_srk_returns_when_handle_exists() {
        // ARRANGE
        let mut dev = MockDevice::new(vec![handle_exists_response(true)]);

        // ACT
        let result = ensure_srk(&mut dev);

        // ASSERT
        assert!(result.is_ok(), "existing SRK should succeed");
    }

    #[test]
    fn seal_with_device_creates_blob_when_srk_exists() {
        // ARRANGE
        let mut create_body = Vec::new();
        create_body.extend_from_slice(&8_u32.to_be_bytes());
        create_body.extend_from_slice(&sized(&[1, 2]));
        create_body.extend_from_slice(&sized(&[3, 4]));
        let mut dev = MockDevice::new(vec![
            handle_exists_response(true),
            body_response(&create_body),
        ]);
        let pcr = [0xAA; 32];

        // ACT
        let result = seal_with_device(&mut dev, &[9], &pcr);

        // ASSERT
        assert!(result.is_ok(), "seal should succeed");
        let sealed = result.expect("seal should succeed");
        assert_eq!(sealed.blob.public(), &[3, 4], "public blob should match");
        assert_eq!(sealed.blob.private(), &[1, 2], "private blob should match");
        assert_eq!(
            sealed.policy_digest,
            compute_policy_digest(&pcr),
            "policy should match"
        );
    }

    #[test]
    fn ensure_srk_creates_and_flushes_missing_srk() {
        // ARRANGE
        let mut create_primary_body = Vec::new();
        create_primary_body.extend_from_slice(&0x8000_0001_u32.to_be_bytes());
        create_primary_body.extend_from_slice(&5_u32.to_be_bytes());
        create_primary_body.extend_from_slice(&sized(&[1]));
        let mut dev = MockDevice::new(vec![
            handle_exists_response(false),
            body_response(&create_primary_body),
            body_response(&[]),
            body_response(&[]),
        ]);

        // ACT
        let result = ensure_srk(&mut dev);

        // ASSERT
        assert!(result.is_ok(), "missing SRK should be created");
    }

    #[test]
    fn seal_with_device_propagates_create_failure() {
        // ARRANGE
        let mut dev = MockDevice::new(vec![handle_exists_response(true)]);

        // ACT
        let result = seal_with_device(&mut dev, &[9], &[0xAA; 32]);

        // ASSERT
        assert!(result.is_err(), "missing create response should fail");
    }

    #[test]
    fn unseal_with_device_loads_policy_and_flushes() {
        // ARRANGE
        let blob =
            SealedBlob::try_new(vec![1], vec![2]).expect("small sealed blob should be valid");
        let load_body = 0x8000_0002_u32.to_be_bytes().to_vec();
        let session_body = 0x0300_0000_u32.to_be_bytes().to_vec();
        let mut unseal_body = Vec::new();
        unseal_body.extend_from_slice(&4_u32.to_be_bytes());
        unseal_body.extend_from_slice(&sized(&[0xAA]));
        let mut dev = MockDevice::new(vec![
            handle_exists_response(true),
            body_response(&load_body),
            body_response(&session_body),
            body_response(&[]),
            body_response(&unseal_body),
            body_response(&[]),
            body_response(&[]),
        ]);

        // ACT
        let result = unseal_with_device(&mut dev, &blob);

        // ASSERT
        let result = result.expect("data should unseal");
        assert_eq!(result.as_slice(), [0xAA].as_slice(), "data should unseal");
    }

    #[test]
    fn unseal_with_device_propagates_load_failure() {
        // ARRANGE
        let blob =
            SealedBlob::try_new(vec![1], vec![2]).expect("small sealed blob should be valid");
        let mut dev = MockDevice::new(vec![handle_exists_response(true)]);

        // ACT
        let result = unseal_with_device(&mut dev, &blob);

        // ASSERT
        assert!(result.is_err(), "missing load response should fail");
    }

    #[test]
    fn unseal_with_device_propagates_policy_failure() {
        // ARRANGE
        let blob =
            SealedBlob::try_new(vec![1], vec![2]).expect("small sealed blob should be valid");
        let load_body = 0x8000_0002_u32.to_be_bytes().to_vec();
        let session_body = 0x0300_0000_u32.to_be_bytes().to_vec();
        let policy_failure = {
            let mut out = Vec::with_capacity(10);
            out.extend_from_slice(&TPM2_ST_NO_SESSIONS.to_be_bytes());
            out.extend_from_slice(&10_u32.to_be_bytes());
            out.extend_from_slice(&0x101_u32.to_be_bytes());
            out
        };
        let mut dev = MockDevice::new(vec![
            handle_exists_response(true),
            body_response(&load_body),
            body_response(&session_body),
            policy_failure,
            body_response(&[]),
            body_response(&[]),
        ]);

        // ACT
        let result = unseal_with_device(&mut dev, &blob);

        // ASSERT
        assert!(result.is_err(), "policy failure should be propagated");
    }

    #[test]
    fn command_structs_provide_metadata() {
        // ASSERT
        assert_eq!(
            CreatePrimaryCommand::TAG,
            0x8002,
            "create primary tag should match"
        );
        assert_eq!(
            StartAuthSessionCommand::COMMAND_CODE,
            0x0000_0176,
            "start auth session code should match"
        );
    }

    #[test]
    fn public_entrypoints_propagate_open_failures() {
        // ARRANGE
        let blob = SealedBlob::try_new(vec![], vec![]).expect("empty blob should be valid");

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
}
