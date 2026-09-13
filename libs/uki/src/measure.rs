//! Streams UKI section measurement records from a PE image file.

use core::mem::{offset_of, size_of};
use core::ops::Range;
use std::fs::File;
use std::io::{self, Read as _};
use std::path::Path;

use object::pe::{
    IMAGE_SIZEOF_SECTION_HEADER, IMAGE_SIZEOF_SHORT_NAME, ImageFileHeader, ImageSectionHeader,
};
use sha2::{Digest as _, Sha256};

use crate::error::{Result, UkiError};
use crate::metadata::DOS_PE_POINTER_OFFSET;
use crate::section::{KERNEL, canonical_name};

const PE_SIGNATURE: [u8; 4] = *b"PE\0\0";
const MAX_SECTIONS: usize = 96;
const SECTION_HEADER_LEN: usize = IMAGE_SIZEOF_SECTION_HEADER;
const TABLE_LEN: usize = MAX_SECTIONS * SECTION_HEADER_LEN;
const CHUNK_LEN: usize = 4096;
const SECTION_COUNT_RANGE: Range<usize> =
    offset_of!(ImageFileHeader, number_of_sections)..offset_of!(ImageFileHeader, time_date_stamp);
const OPTIONAL_LEN_RANGE: Range<usize> = offset_of!(ImageFileHeader, size_of_optional_header)
    ..offset_of!(ImageFileHeader, characteristics);
const NAME_RANGE: Range<usize> = offset_of!(ImageSectionHeader, name)..IMAGE_SIZEOF_SHORT_NAME;
const CONTENT_RANGE: Range<usize> =
    offset_of!(ImageSectionHeader, virtual_size)..offset_of!(ImageSectionHeader, virtual_address);
const DATA_OFFSET_RANGE: Range<usize> = offset_of!(ImageSectionHeader, pointer_to_raw_data)
    ..offset_of!(ImageSectionHeader, pointer_to_relocations);

/// One measured UKI section: canonical name and content digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasuredSection {
    /// Canonical UKI section name.
    pub name: &'static str,
    /// SHA-256 digest of the section content.
    pub hash: [u8; 32],
}

/// Reads the measured UKI sections from the PE image at `path`, in section-table order.
///
/// # Errors
///
/// Returns an error when the image is not a valid PE file, a UKI section is
/// duplicated or missing, or the file cannot be read.
pub fn from_file(path: &Path) -> Result<Vec<MeasuredSection>> {
    let mut file = File::open(path)?;

    let mut dos = [0; DOS_PE_POINTER_OFFSET + size_of::<u32>()];
    file.read_exact(&mut dos)?;
    let pe_offset = u64::from(word(
        &dos,
        DOS_PE_POINTER_OFFSET..DOS_PE_POINTER_OFFSET + size_of::<u32>(),
    )?);
    let dos_len =
        u64::try_from(dos.len()).map_err(|_source| UkiError::Overflow("DOS header length"))?;
    let header_offset = pe_offset
        .checked_sub(dos_len)
        .ok_or(UkiError::InvalidPe("PE header overlaps the DOS header"))?;
    skip(&mut file, header_offset)?;
    let mut position = pe_offset;

    let mut signature = [0; 4];
    file.read_exact(&mut signature)?;
    if signature != PE_SIGNATURE {
        return Err(UkiError::InvalidPe("missing PE signature"));
    }
    position = advance(position, 4)?;

    let mut coff = [0; 20];
    file.read_exact(&mut coff)?;
    position = advance(position, 20)?;
    let count = usize::from(word16(&coff, SECTION_COUNT_RANGE)?);
    if count > MAX_SECTIONS {
        return Err(UkiError::InvalidPe("too many PE sections for a UKI"));
    }
    let optional_len = u64::from(word16(&coff, OPTIONAL_LEN_RANGE)?);
    skip(&mut file, optional_len)?;
    position = advance(position, optional_len)?;

    let table_len = count
        .checked_mul(SECTION_HEADER_LEN)
        .ok_or(UkiError::Overflow("section table length"))?;
    let mut table = [0; TABLE_LEN];
    let used = table
        .get_mut(..table_len)
        .ok_or(UkiError::Overflow("section table length"))?;
    file.read_exact(used)?;
    position = advance(
        position,
        u64::try_from(table_len).map_err(|_source| UkiError::Overflow("section table length"))?,
    )?;

    let mut records: Vec<MeasuredSection> = Vec::new();
    for header in used.as_chunks::<SECTION_HEADER_LEN>().0 {
        let header = parse_header(header)?;
        let offset = u64::from(header.offset);
        let gap = offset.checked_sub(position).ok_or(UkiError::InvalidPe(
            "section data precedes the current position",
        ))?;
        skip(&mut file, gap)?;
        position = offset;

        if let Some(name) = canonical_name(header.name)?
            && header.content > 0
        {
            push_record(&mut records, &mut file, name, header.content)?;
            position = advance(position, u64::from(header.content))?;
        }
    }

    if !records.iter().any(|record| record.name == KERNEL) {
        return Err(UkiError::InvalidPe("missing .kernel section"));
    }

    Ok(records)
}

struct RawHeader {
    name: [u8; 8],
    content: u32,
    offset: u32,
}

fn push_record(
    records: &mut Vec<MeasuredSection>,
    file: &mut File,
    name: &'static str,
    size: u32,
) -> Result<()> {
    if records.iter().any(|record| record.name == name) {
        return Err(UkiError::InvalidPe("duplicate UKI section"));
    }
    let hash = hash_content(file, size)?;
    records.push(MeasuredSection { name, hash });

    Ok(())
}

fn hash_content(file: &mut File, size: u32) -> Result<[u8; 32]> {
    let mut source = file.take(u64::from(size));
    let mut hasher = Sha256::new();
    let mut chunk = [0; CHUNK_LEN];
    loop {
        let limit = usize::try_from(source.limit())
            .map_err(|_source| UkiError::Overflow("section content length"))?;
        if limit == 0 {
            break;
        }
        let take = limit.min(chunk.len());
        let filled = chunk
            .get_mut(..take)
            .ok_or(UkiError::Overflow("chunk length"))?;
        source.read_exact(filled)?;
        hasher.update(filled);
    }

    let mut hash = [0; 32];
    hash.copy_from_slice(hasher.finalize().as_ref());

    Ok(hash)
}

fn skip(file: &mut File, bytes: u64) -> Result<()> {
    let copied = io::copy(&mut file.take(bytes), &mut io::sink())?;
    if copied != bytes {
        return Err(UkiError::InvalidPe("truncated PE image"));
    }

    Ok(())
}

fn advance(position: u64, bytes: u64) -> Result<u64> {
    position
        .checked_add(bytes)
        .ok_or(UkiError::Overflow("file position"))
}

fn word16(bytes: &[u8], range: Range<usize>) -> Result<u16> {
    let raw = bytes
        .get(range)
        .ok_or(UkiError::InvalidPe("truncated PE header"))?;

    Ok(u16::from_le_bytes(raw.try_into().map_err(|_source| {
        UkiError::InvalidPe("truncated PE header")
    })?))
}

fn word(bytes: &[u8], range: Range<usize>) -> Result<u32> {
    let raw = bytes
        .get(range)
        .ok_or(UkiError::InvalidPe("truncated PE header"))?;

    Ok(u32::from_le_bytes(raw.try_into().map_err(|_source| {
        UkiError::InvalidPe("truncated PE header")
    })?))
}

fn parse_header(raw: &[u8; SECTION_HEADER_LEN]) -> Result<RawHeader> {
    Ok(RawHeader {
        name: raw
            .get(NAME_RANGE)
            .ok_or(UkiError::InvalidPe("truncated section header"))?
            .try_into()
            .map_err(|_source| UkiError::InvalidPe("truncated section header"))?,
        content: word(raw, CONTENT_RANGE)?,
        offset: word(raw, DATA_OFFSET_RANGE)?,
    })
}
