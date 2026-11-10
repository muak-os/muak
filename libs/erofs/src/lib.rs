//! Pure-Rust EROFS image writer (mkfs.erofs equivalent).

#![warn(missing_docs)]

extern crate alloc;

mod checked;
mod compress;
pub mod dir;
pub mod error;
mod filecontexts;
mod inode;
pub mod layout;
pub mod source;
mod superblock;
pub mod writer;
mod xattr;

/// Default zstd compression level for EROFS images.
pub const DEFAULT_ZSTD_COMPRESSION_LEVEL: i32 = compress::DEFAULT_ZSTD_COMPRESSION_LEVEL;
/// Block size used throughout EROFS images (4 KiB).
pub const BLOCK_SIZE: u32 = 4096;
/// Slot size: every inode occupies an integer number of 32-byte slots.
pub const SLOT_SIZE: usize = 32;

/// EROFS compression algorithm and level.
pub type Compression = compress::Compression;
/// `SELinux` file context rules for labeling EROFS inodes.
pub type FileContexts = filecontexts::FileContexts;
/// Planned inode layout for the EROFS image.
pub type InodeLayout = layout::Inode;
/// A fully-planned EROFS image.
pub type ImagePlan = layout::ImagePlan;
/// A sized file entry with metadata and a reader.
pub type SizedFile<'a> = source::SizedFile<'a>;

/// Configuration for EROFS image creation.
#[derive(Debug)]
pub struct MkfsConfig<'a> {
    /// Timestamp for reproducible builds (seconds since epoch).
    pub source_date_epoch: u64,
    /// Optional `SELinux` file context rules.
    pub file_contexts: Option<&'a FileContexts>,
    /// Filesystem UUID.
    pub uuid: [u8; 16],
    /// Override UID for all inodes.
    pub force_uid: Option<u16>,
    /// Override GID for all inodes.
    pub force_gid: Option<u16>,
    /// Compression algorithm and level.
    pub compression: Compression,
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
    measure_files: &mut [SizedFile<'_>],
    files: &mut [SizedFile<'_>],
    config: &MkfsConfig<'_>,
) -> error::Result<()> {
    if let Some(level) = config.compression.level() {
        compress::validate_compression_level(level)?;
    }
    let plan = layout::plan(measure_files, config)?;

    writer::image(writer, &plan, files, config)
}
