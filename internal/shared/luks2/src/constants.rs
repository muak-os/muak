//! LUKS2 constants and device-mapper ioctl definitions.

/// LUKS magic bytes: `LUKS\xba\xbe`.
pub const LUKS_MAGIC: [u8; 6] = [0x4c, 0x55, 0x4b, 0x53, 0xba, 0xbe];

/// LUKS2 format version.
pub const LUKS2_VERSION: u16 = 0x0002;

/// Size of the binary header in bytes.
pub const BINARY_HEADER_SIZE: usize = 4096;

/// Default JSON metadata area size (bytes).
pub const DEFAULT_JSON_SIZE: u64 = 12288;

/// Default keyslot binary area size (bytes).
pub const DEFAULT_KEYSLOTS_SIZE: u64 = 16_744_448;

/// Default total header size including both copies + keyslot area (16 MiB).
pub const DEFAULT_HEADER_SIZE: u64 = 16 * 1024 * 1024;

/// Default anti-forensic stripe count.
pub const AF_STRIPES: u32 = 4000;

/// Volume key size for AES-256-XTS (two 256-bit keys).
pub const VOLUME_KEY_SIZE: usize = 64;

/// Default sector size for dm-crypt.
pub const DEFAULT_SECTOR_SIZE: u32 = 4096;

/// Cipher string used for dm-crypt table and LUKS2 metadata.
pub const CIPHER_SPEC: &str = "aes-xts-plain64";

/// Checksum algorithm name stored in the binary header.
pub const CHECKSUM_ALG: &str = "sha256";

/// SHA-256 digest length in bytes.
pub const SHA256_LEN: usize = 32;

/// Offset of the checksum field within the binary header.
pub const CHECKSUM_OFFSET: usize = 376;

/// PBKDF2 iteration count for digest verification.
///
/// Low iteration count is safe because the volume key has full entropy
/// (64 random bytes). The digest only needs to confirm correct decryption,
/// not resist brute-force attacks on weak passwords.
pub const DIGEST_ITERATIONS: u32 = 1_000;

// --- Device-mapper ioctl definitions ---

/// Device-mapper ioctl type byte.
pub const DM_IOCTL_TYPE: u8 = 0xFD;

/// `DM_DEV_CREATE` ioctl number.
pub const DM_DEV_CREATE_NR: u8 = 3;

/// `DM_DEV_SUSPEND` ioctl number (also used for resume).
pub const DM_DEV_SUSPEND_NR: u8 = 6;

/// Device-mapper protocol version we target.
pub const DM_VERSION: [u32; 3] = [4, 0, 0];

/// Maximum name length in `DmIoctl`.
pub const DM_NAME_LEN: usize = 128;

/// Maximum UUID length in `DmIoctl`.
pub const DM_UUID_LEN: usize = 129;

/// `BLKPBSZGET` ioctl to query physical block size.
pub const BLKPBSZGET: u32 = 0x127B;

/// Default offset for the first keyslot area (bytes from start of device).
pub const DEFAULT_KEYSLOT_AREA_OFFSET: u64 = 32768;

/// Default size of a single keyslot area (bytes).
pub const DEFAULT_KEYSLOT_AREA_SIZE: u64 = VOLUME_KEY_SIZE as u64 * AF_STRIPES as u64;
