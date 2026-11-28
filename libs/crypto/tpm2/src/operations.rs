//! High-level TPM2 seal/unseal operations.

use zeroize::Zeroizing;

use crate::blob::Sealed;
use crate::commands::{
    self, CreateCommand, CreatePrimaryCommand, EvictControlCommand, FlushContextCommand,
    HandleExistsCommand, LoadCommand, PolicyPcrCommand, StartAuthSessionCommand, UnsealCommand,
};
use crate::error::Result;
use crate::handles::{HierarchyHandle, PersistentHandle};
use crate::pcr::{Digest, compute_policy_digest};
use crate::transport::Transport;

const SRK_HANDLE: PersistentHandle = PersistentHandle::new(0x8100_0001);

/// Result returned from a seal operation.
pub struct SealResult {
    /// The sealed TPM2 blob.
    pub blob: Sealed,
    /// The policy digest used for sealing.
    pub policy_digest: Digest,
}

/// Seals data to PCR#11 with the given expected PCR value.
///
/// # Errors
///
/// Returns an error if the transport exchange or command execution fails, or if the
/// encoded command exceeds `MAX_COMMAND_SIZE` bytes (plaintext roughly above 930 bytes)
/// with `Tpm2Error::BufferTooLarge`.
pub fn seal<T: Transport>(
    transport: &mut T,
    data: &[u8],
    expected_pcr: &Digest,
) -> Result<SealResult> {
    ensure_srk(transport)?;

    let policy_digest = compute_policy_digest(expected_pcr);
    let (pub_data, priv_data) = commands::execute(
        transport,
        &CreateCommand {
            parent_handle: SRK_HANDLE,
            policy_digest: &policy_digest,
            data,
        },
    )?;

    Ok(SealResult {
        blob: Sealed::try_new(pub_data.to_vec(), priv_data.to_vec())?,
        policy_digest,
    })
}

/// Unseals data using the given nonce and current PCR#11 values.
///
/// # Errors
///
/// Returns an error if the transport exchange, object loading, policy setup, or
/// unsealing fails, or if the encoded command for an oversized sealed blob exceeds
/// `MAX_COMMAND_SIZE` bytes with `Tpm2Error::BufferTooLarge`.
pub fn unseal<T: Transport>(
    transport: &mut T,
    blob: &Sealed,
    nonce: &[u8],
) -> Result<Zeroizing<Vec<u8>>> {
    ensure_srk(transport)?;

    let obj_handle = commands::execute(
        transport,
        &LoadCommand {
            parent_handle: SRK_HANDLE,
            pub_data: blob.public(),
            priv_data: blob.private(),
        },
    )?;
    let session = commands::execute(transport, &StartAuthSessionCommand { nonce })?;

    let result = (|| -> Result<Zeroizing<Vec<u8>>> {
        commands::execute(
            transport,
            &PolicyPcrCommand {
                session_handle: session,
                pcr_digest: &[],
            },
        )?;
        let data = commands::execute(
            transport,
            &UnsealCommand {
                object_handle: obj_handle,
                session_handle: session,
            },
        )?;

        Ok(Zeroizing::new(data.to_vec()))
    })();

    drop(commands::execute(
        transport,
        &FlushContextCommand { handle: session },
    ));
    drop(commands::execute(
        transport,
        &FlushContextCommand { handle: obj_handle },
    ));

    result
}

/// Ensures the SRK exists at the well-known persistent handle.
fn ensure_srk<T: Transport>(transport: &mut T) -> Result<()> {
    if commands::execute(transport, &HandleExistsCommand { handle: SRK_HANDLE })? {
        return Ok(());
    }

    let transient_handle = commands::execute(transport, &CreatePrimaryCommand)?;

    let result = commands::execute(
        transport,
        &EvictControlCommand {
            auth: HierarchyHandle::OWNER,
            object: transient_handle,
            persistent: SRK_HANDLE,
        },
    );
    drop(commands::execute(
        transport,
        &FlushContextCommand {
            handle: transient_handle,
        },
    ));
    result?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::TpmCommand as _;
    use crate::tags::{TPM2_ST_NO_SESSIONS, TPM2_ST_SESSIONS};
    use crate::transport::Transport;

    const TPM2_CAP_HANDLES: u32 = 0x0000_0001;
    const TEST_NONCE: [u8; 32] = [0x55; 32];

    #[derive(Default)]
    struct MockTransport {
        responses: Vec<Vec<u8>>,
        next_response: usize,
    }

    impl MockTransport {
        fn new(responses: Vec<Vec<u8>>) -> Self {
            Self {
                responses,
                next_response: 0,
            }
        }
    }

    impl Transport for MockTransport {
        fn transact(&mut self, _request: &[u8]) -> Result<&[u8]> {
            let response = self
                .responses
                .get(self.next_response)
                .map_or(&[][..], Vec::as_slice);
            self.next_response = self
                .next_response
                .checked_add(1)
                .expect("response index should fit usize");

            Ok(response)
        }
    }

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
        let mut transport = MockTransport::new(vec![handle_exists_response(true)]);

        // ACT
        let result = ensure_srk(&mut transport);

        // ASSERT
        assert!(result.is_ok(), "existing SRK should succeed");
    }

    #[test]
    fn seal_creates_blob_when_srk_exists() {
        // ARRANGE
        let mut create_body = Vec::new();
        create_body.extend_from_slice(&8_u32.to_be_bytes());
        create_body.extend_from_slice(&sized(&[1, 2]));
        create_body.extend_from_slice(&sized(&[3, 4]));
        let mut transport = MockTransport::new(vec![
            handle_exists_response(true),
            body_response(&create_body),
        ]);
        let pcr = [0xAA; 32];

        // ACT
        let result = seal(&mut transport, &[9], &pcr);

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
        let mut transport = MockTransport::new(vec![
            handle_exists_response(false),
            body_response(&create_primary_body),
            body_response(&[]),
            body_response(&[]),
        ]);

        // ACT
        let result = ensure_srk(&mut transport);

        // ASSERT
        assert!(result.is_ok(), "missing SRK should be created");
    }

    #[test]
    fn seal_propagates_create_failure() {
        // ARRANGE
        let mut transport = MockTransport::new(vec![handle_exists_response(true)]);

        // ACT
        let result = seal(&mut transport, &[9], &[0xAA; 32]);

        // ASSERT
        assert!(result.is_err(), "missing create response should fail");
    }

    #[test]
    fn unseal_loads_policy_and_flushes() {
        // ARRANGE
        let blob = Sealed::try_new(vec![1], vec![2]).expect("small sealed blob should be valid");
        let load_body = 0x8000_0002_u32.to_be_bytes().to_vec();
        let session_body = 0x0300_0000_u32.to_be_bytes().to_vec();
        let mut unseal_body = Vec::new();
        unseal_body.extend_from_slice(&4_u32.to_be_bytes());
        unseal_body.extend_from_slice(&sized(&[0xAA]));
        let mut transport = MockTransport::new(vec![
            handle_exists_response(true),
            body_response(&load_body),
            body_response(&session_body),
            body_response(&[]),
            body_response(&unseal_body),
            body_response(&[]),
            body_response(&[]),
        ]);

        // ACT
        let result = unseal(&mut transport, &blob, &TEST_NONCE);

        // ASSERT
        let result = result.expect("data should unseal");
        assert_eq!(result.as_slice(), [0xAA].as_slice(), "data should unseal");
    }

    #[test]
    fn unseal_propagates_load_failure() {
        // ARRANGE
        let blob = Sealed::try_new(vec![1], vec![2]).expect("small sealed blob should be valid");
        let mut transport = MockTransport::new(vec![handle_exists_response(true)]);

        // ACT
        let result = unseal(&mut transport, &blob, &TEST_NONCE);

        // ASSERT
        assert!(result.is_err(), "missing load response should fail");
    }

    #[test]
    fn unseal_propagates_policy_failure() {
        // ARRANGE
        let blob = Sealed::try_new(vec![1], vec![2]).expect("small sealed blob should be valid");
        let load_body = 0x8000_0002_u32.to_be_bytes().to_vec();
        let session_body = 0x0300_0000_u32.to_be_bytes().to_vec();
        let policy_failure = {
            let mut out = Vec::with_capacity(10);
            out.extend_from_slice(&TPM2_ST_NO_SESSIONS.to_be_bytes());
            out.extend_from_slice(&10_u32.to_be_bytes());
            out.extend_from_slice(&0x101_u32.to_be_bytes());
            out
        };
        let mut transport = MockTransport::new(vec![
            handle_exists_response(true),
            body_response(&load_body),
            body_response(&session_body),
            policy_failure,
            body_response(&[]),
            body_response(&[]),
        ]);

        // ACT
        let result = unseal(&mut transport, &blob, &TEST_NONCE);

        // ASSERT
        assert!(result.is_err(), "policy failure should be propagated");
    }

    #[test]
    fn command_structs_provide_metadata() {
        // ASSERT
        assert_eq!(
            CreatePrimaryCommand::TAG,
            TPM2_ST_SESSIONS,
            "create primary tag should match"
        );
        assert_eq!(
            StartAuthSessionCommand::COMMAND_CODE,
            0x0000_0176,
            "start auth session code should match"
        );
    }
}
