//! Compression configuration and defaults for EROFS file data.

/// Default zstd compression level used for compressed EROFS data.
pub const DEFAULT_ZSTD_COMPRESSION_LEVEL: i32 = 3;

/// Compression configuration for EROFS file data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    /// Disable file data compression.
    None,
    /// Compress file data with zstd at the provided level.
    Zstd { level: i32 },
}

impl Compression {
    pub(crate) fn is_enabled(self) -> bool {
        matches!(self, Self::Zstd { .. })
    }

    pub(crate) fn level(self) -> Option<i32> {
        match self {
            Self::None => None,
            Self::Zstd { level } => Some(level),
        }
    }
}

impl Default for Compression {
    fn default() -> Self {
        Self::Zstd {
            level: DEFAULT_ZSTD_COMPRESSION_LEVEL,
        }
    }
}
