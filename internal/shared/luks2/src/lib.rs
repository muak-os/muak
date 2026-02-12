//! Pure-Rust LUKS2 format and open implementation.
//!
//! Provides `format()` to create a LUKS2-encrypted block device and `open()` to
//! decrypt the volume key and set up a dm-crypt mapping via kernel ioctls.
//!
//! Supports AES-256-XTS-plain64 with Argon2id key derivation and PBKDF2-SHA256
//! digest verification.

mod constants;
mod crypto;
mod digest;
mod dm;
mod error;
mod header;
mod keyslot;
mod metadata;

pub use error::{Error, Result};

use ring::rand::SecureRandom;
use zeroize::Zeroize;

use constants::{
    BINARY_HEADER_SIZE, CIPHER_SPEC, DEFAULT_HEADER_SIZE, DEFAULT_JSON_SIZE,
    DEFAULT_KEYSLOT_AREA_OFFSET, VOLUME_KEY_SIZE,
};
use header::Header;
use metadata::Metadata;

/// Formats a block device with LUKS2 encryption.
///
/// Creates the LUKS2 header, generates a random volume key, protects it with
/// the given passphrase via Argon2id, and writes everything to disk. The data
/// segment begins at offset 16 MiB (`DEFAULT_HEADER_SIZE`).
///
/// After formatting, call `open()` to activate the dm-crypt mapping, then
/// create a filesystem on `/dev/mapper/<name>`.
pub fn format(device: &str, passphrase: &[u8], label: &str) -> Result<()> {
    let rng = ring::rand::SystemRandom::new();
    let mut volume_key = vec![0u8; VOLUME_KEY_SIZE];
    rng.fill(&mut volume_key)
        .map_err(|_| Error::InvalidField("random generation failed".into()))?;

    let mut kdf_salt = [0u8; 64];
    rng.fill(&mut kdf_salt)
        .map_err(|_| Error::InvalidField("random generation failed".into()))?;

    let sector_size = dm::detect_sector_size(device);

    let uuid = uuid::Uuid::new_v4().to_string();

    let mut hdr = Header::new(&uuid, label);

    let mut meta = Metadata::new(sector_size);
    meta.add_keyslot("0", &kdf_salt);

    let digest_entry = digest::create(&volume_key, &["0"], &["0"])?;
    meta.digests.insert("0".to_string(), digest_entry);

    let keyslot = meta.keyslots.get("0").ok_or(Error::NoKeyslot)?;
    let keyslot_data = keyslot::encrypt_keyslot(passphrase, &volume_key, keyslot)?;

    let json_buf = meta.serialize(DEFAULT_JSON_SIZE)?;

    let primary_hdr = hdr.serialize(true);
    dm::write_device(device, 0, &primary_hdr)?;
    dm::write_device(device, BINARY_HEADER_SIZE as u64, &json_buf)?;
    dm::write_device(device, DEFAULT_KEYSLOT_AREA_OFFSET, &keyslot_data)?;

    let secondary_hdr = hdr.serialize(false);
    dm::write_device(device, DEFAULT_HEADER_SIZE, &secondary_hdr)?;
    dm::write_device(
        device,
        DEFAULT_HEADER_SIZE + BINARY_HEADER_SIZE as u64,
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
pub fn open(device: &str, name: &str, passphrase: &[u8]) -> Result<()> {
    // Read and parse the primary binary header
    let hdr_buf = dm::read_device(device, 0, BINARY_HEADER_SIZE)?;
    let hdr = Header::parse(&hdr_buf)?;

    // Read and parse JSON metadata
    let json_buf = dm::read_device(
        device,
        BINARY_HEADER_SIZE as u64,
        DEFAULT_JSON_SIZE as usize,
    )?;
    let meta = Metadata::deserialize(&json_buf)?;

    // Try each keyslot until one matches
    let mut volume_key = try_keyslots(device, passphrase, &meta)?;

    // Calculate data segment parameters
    let segment = meta
        .segments
        .get("0")
        .ok_or_else(|| Error::InvalidField("no segment 0".into()))?;

    let data_offset: u64 = segment
        .offset
        .parse()
        .map_err(|_| Error::InvalidField("invalid segment offset".into()))?;

    let sector_size = segment.sector_size;

    // Calculate device size for dm-crypt table
    let dev_size_bytes = device_size(device)?;
    let data_size_bytes = dev_size_bytes.saturating_sub(data_offset);
    let size_sectors = data_size_bytes / 512; // dm-crypt always uses 512-byte sectors for length

    let offset_sectors = data_offset / 512;

    // Build dm-crypt UUID from LUKS UUID
    let dm_uuid = format!("CRYPT-LUKS2-{}", hdr.uuid_str().replace('-', ""));

    dm::dm_crypt_open(&dm::CryptParams {
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
pub fn close(name: &str) -> Result<()> {
    dm::dm_crypt_close(name)
}

/// Attempts to decrypt each keyslot and verify against digests.
fn try_keyslots(device: &str, passphrase: &[u8], meta: &Metadata) -> Result<Vec<u8>> {
    for (slot_id, slot) in &meta.keyslots {
        // Read keyslot binary data from device
        let area_offset: u64 = slot
            .area
            .offset
            .parse()
            .map_err(|_| Error::InvalidField("invalid keyslot area offset".into()))?;
        let area_size: u64 = slot
            .area
            .size
            .parse()
            .map_err(|_| Error::InvalidField("invalid keyslot area size".into()))?;

        let encrypted_data = dm::read_device(device, area_offset, area_size as usize)?;

        // Attempt decryption
        let mut candidate = match keyslot::decrypt_keyslot(passphrase, slot, &encrypted_data) {
            Ok(key) => key,
            Err(_) => continue,
        };

        // Verify against each digest that references this keyslot
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
        .filter(|dig| dig.keyslots.contains(&slot_id.to_string()))
        .any(|dig| matches!(digest::verify(candidate, dig), Ok(true)))
}

/// Returns the total size of a block device in bytes.
fn device_size(device: &str) -> Result<u64> {
    let mut file = std::fs::File::open(device)?;
    let size = std::io::Seek::seek(&mut file, std::io::SeekFrom::End(0))?;
    Ok(size)
}
