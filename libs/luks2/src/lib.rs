//! Pure-Rust LUKS2 format and open implementation.
//!
//! Using AES-256-XTS-plain64 with Argon2id key derivation and PBKDF2-SHA256
//! digest verification.

#![warn(missing_docs)]

mod digest;
mod dm;
mod error;
mod header;
mod keyslot;
mod metadata;
mod pbkdf2;
mod xts;

use error::{Luks2Error as Error, Result};
use header::Header;
use metadata::Metadata;
use zeroize::Zeroize as _;

/// Represents a TPM2 token stored in the LUKS2 JSON metadata.
pub type Tpm2Token = metadata::Tpm2Token;

const BINARY_HEADER_SIZE: usize = 4096;
const BINARY_HEADER_SIZE_U64: u64 = 4096;
const CIPHER_SPEC: &str = "aes-xts-plain64";
const DEFAULT_HEADER_SIZE: u64 = 16 * 1024 * 1024;
const DEFAULT_JSON_SIZE: u64 = 12288;
const DEFAULT_JSON_SIZE_USIZE: usize = 12288;
const DEFAULT_KEYSLOT_AREA_OFFSET: u64 = 32768;
const VOLUME_KEY_SIZE: usize = 64;

/// Formats a block device with LUKS2 encryption.
///
/// Creates the LUKS2 header, generates a random volume key, protects it with
/// the given passphrase via Argon2id, and writes everything to disk. The data
/// segment begins at offset 16 MiB (`DEFAULT_HEADER_SIZE`).
///
/// # Errors
///
/// Returns an error when random generation, metadata construction, or device
/// I/O fails.
pub fn format(device: &str, passphrase: &[u8], label: &str) -> Result<()> {
    let mut volume_key = vec![0_u8; VOLUME_KEY_SIZE];
    getrandom::fill(&mut volume_key)
        .map_err(|_error| Error::InvalidField("random generation failed".into()))?;

    let mut kdf_salt = [0_u8; 64];
    getrandom::fill(&mut kdf_salt)
        .map_err(|_error| Error::InvalidField("random generation failed".into()))?;

    let sector_size = dm::device::detect_sector_size(device);

    let uuid = uuid::Uuid::new_v4().to_string();

    let mut hdr = Header::new(&uuid, label)?;

    let mut meta = Metadata::new(sector_size);
    meta.add_keyslot("0", &kdf_salt);

    let digest_entry = digest::create(&volume_key, &["0"], &["0"])?;
    meta.digests.insert(String::from("0"), digest_entry);

    let keyslot = meta.keyslots.get("0").ok_or(Error::NoKeyslot)?;
    let keyslot_data = keyslot::encrypt_keyslot(passphrase, &volume_key, keyslot)?;

    let json_buf = meta.to_json_buffer(DEFAULT_JSON_SIZE)?;

    let primary_hdr = hdr.serialize(true)?;
    dm::device::write_at(device, 0, &primary_hdr)?;
    dm::device::write_at(device, BINARY_HEADER_SIZE_U64, &json_buf)?;
    dm::device::write_at(device, DEFAULT_KEYSLOT_AREA_OFFSET, &keyslot_data)?;

    let secondary_hdr = hdr.serialize(false)?;
    dm::device::write_at(device, DEFAULT_HEADER_SIZE, &secondary_hdr)?;
    dm::device::write_at(
        device,
        DEFAULT_HEADER_SIZE.saturating_add(BINARY_HEADER_SIZE_U64),
        &json_buf,
    )?;

    volume_key.zeroize();
    kdf_salt.zeroize();

    Ok(())
}

/// Opens (activates) a LUKS2-encrypted device.
///
/// Reads the LUKS2 header, attempts to decrypt the volume key using the
/// passphrase against each keyslot, verifies the key against the stored
/// digest, and sets up a dm-crypt mapping at `/dev/mapper/<name>`.
///
/// # Errors
///
/// Returns an error when headers or metadata are invalid, the passphrase does
/// not match, or the device-mapper setup fails.
pub fn open(device: &str, name: &str, passphrase: &[u8]) -> Result<()> {
    let hdr_buf = dm::device::read_at(device, 0, BINARY_HEADER_SIZE)?;
    let hdr = Header::parse(&hdr_buf)?;

    let json_buf = dm::device::read_at(device, BINARY_HEADER_SIZE_U64, DEFAULT_JSON_SIZE_USIZE)?;
    let meta = Metadata::from_json_buffer(&json_buf)?;

    let mut volume_key = try_keyslots(device, passphrase, &meta)?;

    let segment = meta
        .segments
        .get("0")
        .ok_or_else(|| Error::InvalidField("no segment 0".into()))?;

    let data_offset: u64 = segment
        .offset
        .parse()
        .map_err(|_error| Error::InvalidField("invalid segment offset".into()))?;

    let sector_size = segment.sector_size;

    let dev_size_bytes = device_size(device)?;
    let data_size_bytes = dev_size_bytes.saturating_sub(data_offset);
    let size_sectors = data_size_bytes >> 9;

    let offset_sectors = data_offset >> 9;

    let dm_uuid = format!("CRYPT-LUKS2-{}", hdr.uuid_str().replace('-', ""));

    dm::crypt::open(&dm::crypt::CryptParams {
        name,
        dm_uuid: &dm_uuid,
        backing_device: device,
        volume_key: &volume_key,
        cipher: CIPHER_SPEC,
        offset_sectors,
        size_sectors,
        sector_size,
    })?;

    volume_key.zeroize();
    Ok(())
}

/// Closes (deactivates) a dm-crypt mapping.
///
/// Removes the device-mapper device identified by `name`, making
/// `/dev/mapper/<name>` unavailable. The backing device is not modified.
///
/// The device should be unmounted before calling this function.
///
/// # Errors
///
/// Returns an error when the device-mapper mapping cannot be removed.
pub fn close(name: &str) -> Result<()> {
    dm::crypt::close(name)
}

/// Attempts to decrypt each keyslot and verify against digests.
fn try_keyslots(device: &str, passphrase: &[u8], meta: &Metadata) -> Result<Vec<u8>> {
    let mut keyslots = meta.keyslots.iter().collect::<Vec<_>>();
    keyslots.sort_by_key(|&(slot_id, _)| slot_id);

    for (slot_id, slot) in keyslots {
        let area_offset: u64 = slot
            .area
            .offset
            .parse()
            .map_err(|_error| Error::InvalidField("invalid keyslot area offset".into()))?;
        let area_size: u64 = slot
            .area
            .size
            .parse()
            .map_err(|_error| Error::InvalidField("invalid keyslot area size".into()))?;

        let area_size = usize::try_from(area_size)
            .map_err(|_error| Error::InvalidField("invalid keyslot area size".into()))?;

        let encrypted_data = dm::device::read_at(device, area_offset, area_size)?;

        let Ok(mut candidate) = keyslot::decrypt_keyslot(passphrase, slot, &encrypted_data) else {
            continue;
        };

        if verify_candidate(&candidate, slot_id, meta) {
            return Ok(candidate);
        }

        candidate.zeroize();
    }

    Err(Error::WrongPassphrase)
}

/// Checks whether a volume key candidate matches any digest for the given keyslot.
fn verify_candidate(candidate: &[u8], slot_id: &str, meta: &Metadata) -> bool {
    meta.digests
        .values()
        .filter(|digest| {
            digest
                .keyslots
                .iter()
                .any(|digest_slot| digest_slot == slot_id)
        })
        .any(|dig| matches!(digest::verify(candidate, dig), Ok(true)))
}

/// Returns the total size of a block device in bytes.
fn device_size(device: &str) -> Result<u64> {
    let mut file = std::fs::File::open(device)?;
    let size = std::io::Seek::seek(&mut file, std::io::SeekFrom::End(0))?;
    Ok(size)
}

/// Reads the TPM2 token from a LUKS2 device header.
///
/// # Errors
///
/// Returns an error when the JSON metadata cannot be read or does not contain a
/// TPM2 token.
pub fn read_tpm2_token(device: &str) -> Result<Tpm2Token> {
    let json_buf = dm::device::read_at(device, BINARY_HEADER_SIZE_U64, DEFAULT_JSON_SIZE_USIZE)?;
    let meta = Metadata::from_json_buffer(&json_buf)?;
    meta.get_tpm2_token()
}

/// Writes a TPM2 token to both copies of the LUKS2 JSON metadata.
///
/// # Errors
///
/// Returns an error when metadata cannot be read, updated, serialized, or
/// written back to disk.
pub fn write_tpm2_token(device: &str, token: &Tpm2Token) -> Result<()> {
    let json_buf = dm::device::read_at(device, BINARY_HEADER_SIZE_U64, DEFAULT_JSON_SIZE_USIZE)?;
    let mut meta = Metadata::from_json_buffer(&json_buf)?;
    meta.set_tpm2_token(token)?;

    let new_json = meta.to_json_buffer(DEFAULT_JSON_SIZE)?;

    dm::device::write_at(device, BINARY_HEADER_SIZE_U64, &new_json)?;
    dm::device::write_at(
        device,
        DEFAULT_HEADER_SIZE.saturating_add(BINARY_HEADER_SIZE_U64),
        &new_json,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::Digest;
    use crate::metadata::Tpm2Token;

    fn create_test_device(size: u64) -> tempfile::NamedTempFile {
        let file = tempfile::NamedTempFile::new().unwrap();
        file.as_file().set_len(size).unwrap();
        file
    }

    fn path_str(file: &tempfile::NamedTempFile) -> &str {
        file.path().to_str().unwrap()
    }

    fn create_test_digest(volume_key: &[u8]) -> Digest {
        digest::create(volume_key, &["0"], &["0"]).unwrap()
    }

    fn create_test_metadata() -> Metadata {
        let mut meta = Metadata::new(4096);
        meta.add_keyslot("0", &[0x42_u8; 64]);
        meta
    }

    #[test]
    fn verify_candidate_correct_key() {
        // ARRANGE
        let volume_key = vec![0xAB_u8; 64];
        let mut meta = create_test_metadata();

        let digest = create_test_digest(&volume_key);
        meta.digests.insert("0".to_owned(), digest);

        // ACT
        let result = verify_candidate(&volume_key, "0", &meta);

        // ASSERT
        assert!(result);
    }

    #[test]
    fn verify_candidate_wrong_key() {
        // ARRANGE
        let correct_key = vec![0xAB_u8; 64];
        let wrong_key = vec![0xCD_u8; 64];
        let mut meta = create_test_metadata();

        let digest = create_test_digest(&correct_key);
        meta.digests.insert("0".to_owned(), digest);

        // ACT
        let result = verify_candidate(&wrong_key, "0", &meta);

        // ASSERT
        assert!(!result);
    }

    #[test]
    fn verify_candidate_no_matching_digest() {
        // ARRANGE
        let volume_key = vec![0xAB_u8; 64];
        let meta = create_test_metadata();

        // ACT
        let result = verify_candidate(&volume_key, "0", &meta);

        // ASSERT
        assert!(!result);
    }

    #[test]
    fn verify_candidate_wrong_keyslot() {
        // ARRANGE
        let volume_key = vec![0xAB_u8; 64];
        let mut meta = create_test_metadata();

        let digest = create_test_digest(&volume_key);
        meta.digests.insert("0".to_owned(), digest);

        // ACT
        let result = verify_candidate(&volume_key, "2", &meta);

        // ASSERT
        assert!(!result);
    }

    #[test]
    fn verify_candidate_multiple_digests_one_matches() {
        // ARRANGE
        let volume_key = vec![0xAB_u8; 64];
        let mut meta = create_test_metadata();
        meta.add_keyslot("1", &[0x43_u8; 64]);

        let digest0 = create_test_digest(&volume_key);
        meta.digests.insert("0".to_owned(), digest0);

        let other_key = vec![0xCD_u8; 64];
        let digest1 = create_test_digest(&other_key);
        meta.digests.insert("1".to_owned(), digest1);

        // ACT
        let result = verify_candidate(&volume_key, "0", &meta);
        assert!(result);

        let result = verify_candidate(&volume_key, "1", &meta);
        // ASSERT
        assert!(!result);
    }

    #[test]
    fn error_from_io() {
        // ARRANGE
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "test");

        // ACT
        let err: Error = io_err.into();

        // ASSERT
        assert!(matches!(err, Error::Io(_)));
    }

    #[test]
    fn error_from_json() {
        // ARRANGE
        let result: core::result::Result<serde_json::Value, _> = serde_json::from_str("invalid");
        let json_err = result.unwrap_err();

        // ACT
        let err: Error = json_err.into();

        // ASSERT
        assert!(matches!(err, Error::Json(_)));
    }

    #[test]
    fn device_size_regular_file() {
        // ARRANGE
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let data = vec![0_u8; 4096];
        std::io::Write::write_all(&mut file, &data).unwrap();

        // ACT
        let size = device_size(file.path().to_str().unwrap()).unwrap();

        // ASSERT
        assert_eq!(size, 4096);
    }

    #[test]
    fn device_size_nonexistent_returns_error() {
        // ACT & ASSERT
        let result = device_size("/nonexistent/dev/not_real");
        result.unwrap_err();
    }

    #[test]
    fn format_and_read_tpm2_token_round_trip() {
        // ARRANGE
        let device = create_test_device(DEFAULT_HEADER_SIZE + 1024 * 1024);
        let token = Tpm2Token {
            r#type: String::from("tpm2"),
            keyslots: vec![String::from("0")],
            tpm2_pcrs: vec![7, 11],
            tpm2_hash_alg: String::from("sha256"),
            tpm2_blob: String::from("blob"),
            tpm2_policy_hash: String::from("policy"),
        };

        // ACT
        format(path_str(&device), b"passphrase", "label").unwrap();
        write_tpm2_token(path_str(&device), &token).unwrap();
        let read_back = read_tpm2_token(path_str(&device)).unwrap();

        // ASSERT
        assert_eq!(read_back.r#type, token.r#type);
        assert_eq!(read_back.keyslots, token.keyslots);
        assert_eq!(read_back.tpm2_pcrs, token.tpm2_pcrs);
        assert_eq!(read_back.tpm2_blob, token.tpm2_blob);
    }

    #[test]
    fn write_tpm2_token_replaces_existing_token() {
        // ARRANGE
        let device = create_test_device(DEFAULT_HEADER_SIZE + 1024 * 1024);
        let first = Tpm2Token {
            r#type: String::from("tpm2"),
            keyslots: vec![String::from("0")],
            tpm2_pcrs: vec![11],
            tpm2_hash_alg: String::from("sha256"),
            tpm2_blob: String::from("first"),
            tpm2_policy_hash: String::from("policy-a"),
        };
        let second = Tpm2Token {
            r#type: String::from("tpm2"),
            keyslots: vec![String::from("0")],
            tpm2_pcrs: vec![11],
            tpm2_hash_alg: String::from("sha256"),
            tpm2_blob: String::from("second"),
            tpm2_policy_hash: String::from("policy-b"),
        };

        // ACT
        format(path_str(&device), b"passphrase", "label").unwrap();
        write_tpm2_token(path_str(&device), &first).unwrap();
        write_tpm2_token(path_str(&device), &second).unwrap();
        let read_back = read_tpm2_token(path_str(&device)).unwrap();

        // ASSERT
        assert_eq!(read_back.tpm2_blob, "second");
        assert_eq!(read_back.tpm2_policy_hash, "policy-b");
    }

    #[test]
    fn read_tpm2_token_returns_error_when_missing() {
        // ARRANGE
        let device = create_test_device(DEFAULT_HEADER_SIZE + 1024 * 1024);
        format(path_str(&device), b"passphrase", "label").unwrap();

        // ACT
        let result = read_tpm2_token(path_str(&device));

        // ASSERT
        assert!(matches!(result, Err(Error::NoTpm2Token)));
    }

    #[test]
    fn open_returns_wrong_passphrase_for_valid_formatted_device() {
        // ARRANGE
        let device = create_test_device(DEFAULT_HEADER_SIZE + 1024 * 1024);
        format(path_str(&device), b"correct", "label").unwrap();

        // ACT
        let result = open(path_str(&device), "mapper-name", b"wrong");

        // ASSERT
        assert!(matches!(result, Err(Error::WrongPassphrase)));
    }

    #[test]
    fn open_returns_error_when_segment_offset_is_invalid() {
        // ARRANGE
        let device = create_test_device(DEFAULT_HEADER_SIZE + 1024 * 1024);
        format(path_str(&device), b"passphrase", "label").unwrap();

        let json_buf = dm::device::read_at(
            path_str(&device),
            BINARY_HEADER_SIZE_U64,
            DEFAULT_JSON_SIZE_USIZE,
        )
        .unwrap();
        let mut metadata = Metadata::from_json_buffer(&json_buf).unwrap();
        metadata.segments.get_mut("0").unwrap().offset = String::from("invalid");
        let new_json = metadata.to_json_buffer(DEFAULT_JSON_SIZE).unwrap();
        dm::device::write_at(path_str(&device), BINARY_HEADER_SIZE_U64, &new_json).unwrap();

        // ACT
        let result = open(path_str(&device), "mapper-name", b"passphrase");

        // ASSERT
        assert!(
            matches!(result, Err(Error::InvalidField(field)) if field == "invalid segment offset")
        );
    }

    #[test]
    fn close_propagates_device_mapper_error() {
        // ACT
        let result = close("definitely-not-a-real-mapper");

        // ASSERT
        assert!(matches!(result, Err(Error::DeviceMapper(_))));
    }

    #[test]
    fn open_returns_error_for_missing_keyslot_segment() {
        // ARRANGE
        let device = create_test_device(DEFAULT_HEADER_SIZE + 1024 * 1024);
        format(path_str(&device), b"passphrase", "label").unwrap();

        let json_buf = dm::device::read_at(
            path_str(&device),
            BINARY_HEADER_SIZE_U64,
            DEFAULT_JSON_SIZE_USIZE,
        )
        .unwrap();
        let mut metadata = Metadata::from_json_buffer(&json_buf).unwrap();
        metadata.segments.remove("0");
        let new_json = metadata.to_json_buffer(DEFAULT_JSON_SIZE).unwrap();
        dm::device::write_at(path_str(&device), BINARY_HEADER_SIZE_U64, &new_json).unwrap();

        // ACT
        let result = open(path_str(&device), "mapper-name", b"passphrase");

        // ASSERT
        assert!(matches!(result, Err(Error::InvalidField(field)) if field == "no segment 0"));
    }

    #[test]
    fn write_tpm2_token_returns_error_for_invalid_json() {
        // ARRANGE
        let device = create_test_device(DEFAULT_HEADER_SIZE + 1024 * 1024);
        format(path_str(&device), b"passphrase", "label").unwrap();
        dm::device::write_at(path_str(&device), BINARY_HEADER_SIZE_U64, b"not-json").unwrap();
        let token = Tpm2Token {
            r#type: String::from("tpm2"),
            keyslots: vec![String::from("0")],
            tpm2_pcrs: vec![11],
            tpm2_hash_alg: String::from("sha256"),
            tpm2_blob: String::from("blob"),
            tpm2_policy_hash: String::from("policy"),
        };

        // ACT
        let result = write_tpm2_token(path_str(&device), &token);

        // ASSERT
        assert!(matches!(result, Err(Error::Json(_))));
    }

    #[test]
    fn open_returns_error_for_missing_device() {
        // ACT
        let result = open("/nonexistent/luks2-device", "mapper-name", b"passphrase");

        // ASSERT
        assert!(matches!(result, Err(Error::Io(_))));
    }

    #[test]
    fn read_tpm2_token_returns_error_for_invalid_json() {
        // ARRANGE
        let device = create_test_device(DEFAULT_HEADER_SIZE + 1024 * 1024);
        format(path_str(&device), b"passphrase", "label").unwrap();
        dm::device::write_at(path_str(&device), BINARY_HEADER_SIZE_U64, b"not-json").unwrap();

        // ACT
        let result = read_tpm2_token(path_str(&device));

        // ASSERT
        assert!(matches!(result, Err(Error::Json(_))));
    }
}
