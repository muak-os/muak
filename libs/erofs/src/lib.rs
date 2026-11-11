//! Pure-Rust EROFS image writer (mkfs.erofs equivalent).

#![warn(missing_docs)]

extern crate alloc;

mod checked;
pub mod compress;
pub mod dir;
pub mod error;
pub mod filecontexts;
mod inode;
pub mod layout;
pub mod source;
mod superblock;
pub mod writer;
mod xattr;

/// Block size used throughout EROFS images (4 KiB).
pub const BLOCK_SIZE: u32 = 4096;
/// Slot size: every inode occupies an integer number of 32-byte slots.
pub const SLOT_SIZE: usize = 32;

/// Configuration for EROFS image creation.
#[derive(Debug)]
pub struct MkfsConfig<'a> {
    /// Timestamp for reproducible builds (seconds since epoch).
    pub source_date_epoch: u64,
    /// Optional `SELinux` file context rules.
    pub file_contexts: Option<&'a filecontexts::FileContexts>,
    /// Filesystem UUID.
    pub uuid: [u8; 16],
    /// Override UID for all inodes.
    pub force_uid: Option<u16>,
    /// Override GID for all inodes.
    pub force_gid: Option<u16>,
    /// Compression algorithm and level.
    pub compression: compress::Compression,
}

/// Validate a zstd compression level for EROFS images.
///
/// # Errors
///
/// Returns an [`error::ErofsError::InvalidCompressionLevel`] error when `level`
/// is neither `0` nor within the zstd-supported compression level range.
pub fn validate_compression_level(level: i32) -> error::Result<i32> {
    compress::validate_compression_level(level)
}

/// Build an EROFS filesystem image from sized file entries.
///
/// # Errors
///
/// Returns an error when entries are invalid, compression settings are invalid,
/// filesystem metadata cannot be read, or the image cannot be serialized.
pub fn mkfs<W: std::io::Write>(
    writer: &mut W,
    measure_files: &mut [source::SizedFile<'_>],
    files: &mut [source::SizedFile<'_>],
    config: &MkfsConfig<'_>,
) -> error::Result<()> {
    if let Some(level) = config.compression.level() {
        compress::validate_compression_level(level)?;
    }
    let plan = layout::plan(measure_files, config)?;

    writer::image(writer, &plan, files, config)
}
