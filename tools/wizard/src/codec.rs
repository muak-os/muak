//! Transport codecs for compressible output artifacts.

use std::io::{self, Write};

use flate2::write::GzEncoder;

use crate::error::{Result, WizardError};

/// The zstd compression level used by [`Codec::Zstd`].
const ZSTD_LEVEL: i32 = 6;

/// Transport codec applied to compressible artifacts.
///
/// The default is zstd, declared once via `#[default]` below.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Codec {
    /// No transport encoding.
    None,
    /// zstd compression.
    #[default]
    Zstd,
    /// gzip compression.
    Gzip,
}

impl Codec {
    /// Returns the filename extension for this codec, without the dot.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Zstd => "zst",
            Self::Gzip => "gz",
        }
    }

    /// Returns the URL/CLI token for this codec (`none`, `zst`, or `gz`).
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Zstd => "zst",
            Self::Gzip => "gz",
        }
    }

    /// Returns true when this codec transforms bytes.
    #[must_use]
    pub const fn compresses(&self) -> bool {
        !matches!(self, Self::None)
    }

    /// Parses a codec from its CLI/URL token (`none`, `zst`, or `gz`).
    ///
    /// # Errors
    ///
    /// Returns a [`WizardError::RequestValidation`] for any token other than
    /// `none`, `zst`, or `gz`.
    pub fn parse(input: &str) -> Result<Self> {
        match input {
            "none" => Ok(Self::None),
            "zst" => Ok(Self::Zstd),
            "gz" => Ok(Self::Gzip),
            other => Err(WizardError::RequestValidation(format!(
                "unknown codec: {other}"
            ))),
        }
    }

    /// Wraps `inner` in this codec's encoder.
    ///
    /// # Errors
    ///
    /// Returns an error when the codec encoder cannot be initialized.
    pub fn encoder<W: Write>(self, inner: W) -> io::Result<Encoder<W>> {
        match self {
            Self::None => Ok(Encoder::Plain(inner)),
            Self::Zstd => {
                let encoder = zstd::Encoder::new(inner, ZSTD_LEVEL)
                    .map_err(|e| io::Error::other(format!("initialize zstd encoder: {e}")))?;

                Ok(Encoder::Zstd(encoder))
            }
            Self::Gzip => Ok(Encoder::Gzip(GzEncoder::new(
                inner,
                flate2::Compression::default(),
            ))),
        }
    }
}

/// A codec's streaming encoder over any sink.
pub enum Encoder<W: Write> {
    /// Pass-through sink.
    Plain(W),
    /// zstd encoder.
    Zstd(zstd::Encoder<'static, W>),
    /// gzip encoder.
    Gzip(GzEncoder<W>),
}

impl<W: Write> Encoder<W> {
    /// Finishes the stream and returns the underlying sink.
    ///
    /// # Errors
    ///
    /// Returns an error when the codec trailer cannot be written.
    pub fn finish(self) -> io::Result<W> {
        match self {
            Self::Plain(inner) => Ok(inner),
            Self::Zstd(encoder) => encoder.finish(),
            Self::Gzip(encoder) => encoder.finish(),
        }
    }
}

impl<W: Write> Write for Encoder<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match *self {
            Self::Plain(ref mut inner) => inner.write(buf),
            Self::Zstd(ref mut encoder) => encoder.write(buf),
            Self::Gzip(ref mut encoder) => encoder.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match *self {
            Self::Plain(ref mut inner) => inner.flush(),
            Self::Zstd(ref mut encoder) => encoder.flush(),
            Self::Gzip(ref mut encoder) => encoder.flush(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use super::*;

    #[test]
    fn parse_roundtrips_known_tokens() {
        // ARRANGE & ACT & ASSERT
        assert_eq!(Codec::parse("none").expect("none"), Codec::None);
        assert_eq!(Codec::parse("zst").expect("zst"), Codec::Zstd);
        assert_eq!(Codec::parse("gz").expect("gz"), Codec::Gzip);
    }

    #[test]
    fn parse_rejects_unknown_tokens() {
        // ARRANGE & ACT
        let error = Codec::parse("xz").expect_err("unknown codec must fail");

        // ASSERT
        assert!(error.to_string().contains("unknown codec: xz"));
    }

    #[test]
    fn extensions_match_parse_tokens() {
        // ARRANGE & ACT & ASSERT
        assert_eq!(Codec::None.extension(), "");
        assert_eq!(Codec::Zstd.extension(), "zst");
        assert_eq!(Codec::Gzip.extension(), "gz");
        assert!(!Codec::None.compresses());
        assert!(Codec::Zstd.compresses());
    }

    #[test]
    fn zstd_encoder_writes_zstd_framed_output() {
        // ARRANGE
        let mut sink = io::Cursor::new(Vec::new());

        // ACT
        {
            let mut encoder = Codec::Zstd.encoder(&mut sink).expect("encoder");
            encoder.write_all(b"payload").expect("write");
            encoder.finish().expect("finish");
        }

        // ASSERT
        let bytes = sink.into_inner();
        assert_eq!(
            bytes.get(..4),
            Some(&[0x28, 0xb5, 0x2f, 0xfd][..]),
            "zstd magic"
        );
        let mut decoder = zstd::stream::read::Decoder::new(bytes.as_slice()).expect("decoder");
        let mut out = Vec::new();
        decoder.read_to_end(&mut out).expect("decode");
        assert_eq!(out, b"payload");
    }

    #[test]
    fn gzip_encoder_writes_gzip_framed_output() {
        // ARRANGE
        let mut sink = io::Cursor::new(Vec::new());

        // ACT
        {
            let mut encoder = Codec::Gzip.encoder(&mut sink).expect("encoder");
            encoder.write_all(b"payload").expect("write");
            encoder.finish().expect("finish");
        }

        // ASSERT
        let bytes = sink.into_inner();
        assert_eq!(bytes.get(..2), Some(&[0x1f, 0x8b][..]), "gzip magic");
        let mut decoder = flate2::read::GzDecoder::new(bytes.as_slice());
        let mut out = Vec::new();
        decoder.read_to_end(&mut out).expect("decode");
        assert_eq!(out, b"payload");
    }

    #[test]
    fn plain_encoder_passes_bytes_through() {
        // ARRANGE
        let mut sink = io::Cursor::new(Vec::new());

        // ACT
        {
            let mut encoder = Codec::None.encoder(&mut sink).expect("encoder");
            encoder.write_all(b"payload").expect("write");
            encoder.finish().expect("finish");
        }

        // ASSERT
        assert_eq!(sink.into_inner(), b"payload");
    }
}
