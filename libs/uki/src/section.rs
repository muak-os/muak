//! Parsing UKI sections from a PE image.

use core::str;

use object::LittleEndian as LE;
use object::pe::ImageSectionHeader;
use object::read::pe::PeFile64;

use crate::error::{Result, UkiError};

/// PE section name for the kernel image.
pub const KERNEL: &str = ".kernel";
/// PE section name for the initramfs.
pub const INITRD: &str = ".initrd";
/// PE section name for the kernel command line.
pub const CMDLINE: &str = ".cmdline";
/// UKI sections in canonical order.
pub const CANONICAL_ORDER: [&str; 3] = [CMDLINE, INITRD, KERNEL];

/// Parsed UKI sections from a PE image.
#[derive(Debug)]
pub struct Sections<'a> {
    /// Kernel image bytes.
    pub kernel: &'a [u8],
    /// Optional initramfs bytes.
    pub initrd: Option<&'a [u8]>,
    /// Optional command line bytes.
    pub cmdline: Option<&'a [u8]>,
}

impl<'a> Sections<'a> {
    /// Parses UKI sections from a PE image.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the image is not a valid PE file, a section is malformed,
    /// or the required `.kernel` section is missing.
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        if data.len() < 0x40 {
            return Err(UkiError::InvalidPe("file too small"));
        }
        let pe =
            PeFile64::parse(data).map_err(|_source| UkiError::InvalidPe("invalid PE format"))?;

        let items: Vec<_> = pe
            .section_table()
            .iter()
            .filter_map(|section| uki_section_data(data, section).transpose())
            .collect::<Result<Vec<_>>>()?;

        let mut kernel = None::<&'a [u8]>;
        let mut initrd = None::<&'a [u8]>;
        let mut cmdline = None::<&'a [u8]>;

        for (name, section_data) in items {
            set_uki_section(name, section_data, &mut kernel, &mut initrd, &mut cmdline)?;
        }

        Ok(Sections {
            kernel: kernel.ok_or(UkiError::InvalidPe("missing .kernel section"))?,
            initrd,
            cmdline,
        })
    }

    /// Returns an iterator over sections to measure, in [`CANONICAL_ORDER`] order.
    pub fn iter_sections(&self) -> impl Iterator<Item = (&'static str, &'a [u8])> {
        let mut sections = [
            (KERNEL, Some(self.kernel)),
            (CMDLINE, self.cmdline),
            (INITRD, self.initrd),
        ];
        sections.sort_by_key(|&(name, _data)| canonical_rank(name));

        sections
            .into_iter()
            .filter_map(|(name, data)| data.map(|section_data| (name, section_data)))
    }
}

/// Returns the canonical position of a UKI section name in [`CANONICAL_ORDER`]; unknown names sort last.
#[must_use]
pub fn canonical_rank(name: &str) -> usize {
    CANONICAL_ORDER
        .iter()
        .position(|canonical| *canonical == name)
        .unwrap_or(CANONICAL_ORDER.len())
}

/// Maps a raw PE section name to its canonical UKI section name.
///
/// # Errors
///
/// Returns an error when the name is not valid UTF-8.
pub(crate) fn canonical_name(raw: [u8; 8]) -> Result<Option<&'static str>> {
    let text =
        str::from_utf8(&raw).map_err(|_source| UkiError::InvalidPe("invalid section name"))?;

    let name = text.trim_end_matches('\0');

    Ok(CANONICAL_ORDER
        .into_iter()
        .find(|canonical| *canonical == name))
}

fn set_uki_section<'a>(
    name: &'static str,
    section_data: &'a [u8],
    kernel: &mut Option<&'a [u8]>,
    initrd: &mut Option<&'a [u8]>,
    cmdline: &mut Option<&'a [u8]>,
) -> Result<()> {
    match name {
        KERNEL => *kernel = Some(section_data),
        INITRD => *initrd = Some(section_data),
        CMDLINE => *cmdline = Some(section_data),
        _ => return Err(UkiError::InvalidPe("unexpected UKI section")),
    }

    Ok(())
}

fn uki_section_data<'a>(
    data: &'a [u8],
    section: &ImageSectionHeader,
) -> Result<Option<(&'static str, &'a [u8])>> {
    let Some(name) = canonical_name(section.name)? else {
        return Ok(None);
    };

    let size = usize::try_from(section.virtual_size.get(LE))
        .map_err(|_source| UkiError::Overflow("section size"))?;
    if size == 0 {
        return Ok(None);
    }

    let rva = usize::try_from(section.virtual_address.get(LE))
        .map_err(|_source| UkiError::Overflow("section RVA"))?;
    let end = rva
        .checked_add(size)
        .ok_or(UkiError::Overflow("section bounds"))?;
    let section_data = data
        .get(rva..end)
        .ok_or(UkiError::InvalidPe("section data out of bounds"))?;

    Ok(Some((name, section_data)))
}
