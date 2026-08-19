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

    /// Returns an iterator over sections to measure, in spec canonical order.
    pub fn iter_sections(&self) -> impl Iterator<Item = (&'static str, &'a [u8])> {
        [
            (KERNEL, Some(self.kernel)),
            (CMDLINE, self.cmdline),
            (INITRD, self.initrd),
        ]
        .into_iter()
        .filter_map(|(name, data)| data.map(|section_data| (name, section_data)))
    }
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
    let name = str::from_utf8(&section.name)
        .map_err(|_source| UkiError::InvalidPe("invalid section name"))?
        .trim_end_matches('\0');
    let Some(name) = canonical_uki_section_name(name) else {
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

fn canonical_uki_section_name(name: &str) -> Option<&'static str> {
    match name {
        KERNEL => Some(KERNEL),
        INITRD => Some(INITRD),
        CMDLINE => Some(CMDLINE),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE_ALIGN: usize = 0x200;
    const NT_OFFSET: usize = 0x40;

    fn write_bytes(buf: &mut [u8], offset: usize, data: &[u8]) {
        let end = offset
            .checked_add(data.len())
            .expect("write_bytes offset overflow");
        buf.get_mut(offset..end)
            .expect("write_bytes range")
            .copy_from_slice(data);
    }

    fn write_u16(buf: &mut [u8], offset: usize, value: u16) {
        write_bytes(buf, offset, &value.to_le_bytes());
    }

    fn write_u32(buf: &mut [u8], offset: usize, value: u32) {
        write_bytes(buf, offset, &value.to_le_bytes());
    }

    fn build_test_pe() -> Vec<u8> {
        let nt_off = NT_OFFSET;
        let file_hdr = nt_off.checked_add(4).expect("file header");
        let opt_off = file_hdr.checked_add(20).expect("opt header");

        let hdr_size = FILE_ALIGN;
        let mut data = vec![0_u8; hdr_size];

        write_bytes(&mut data, 0, b"MZ");
        write_u32(&mut data, 0x3C, u32::try_from(nt_off).expect("nt offset"));
        write_bytes(&mut data, nt_off, b"PE");

        write_u16(&mut data, file_hdr, 0x8664);
        write_u16(
            &mut data,
            file_hdr.checked_add(16).expect("size of opt hdr"),
            0xF0,
        );
        write_u16(
            &mut data,
            file_hdr.checked_add(18).expect("characteristics"),
            0x0002,
        );

        write_u16(&mut data, opt_off, 0x020B);
        write_u32(
            &mut data,
            opt_off.checked_add(16).expect("entry point"),
            u32::try_from(FILE_ALIGN).expect("file align"),
        );
        write_u64(
            &mut data,
            opt_off.checked_add(24).expect("image base"),
            0x0000_0000_0400_0000,
        );
        write_u32(
            &mut data,
            opt_off.checked_add(32).expect("section align"),
            u32::try_from(FILE_ALIGN).expect("section align"),
        );
        write_u32(
            &mut data,
            opt_off.checked_add(36).expect("file align"),
            u32::try_from(FILE_ALIGN).expect("file align 2"),
        );
        write_u16(
            &mut data,
            opt_off.checked_add(44).expect("major image version"),
            1,
        );
        write_u32(
            &mut data,
            opt_off.checked_add(56).expect("size of image"),
            u32::try_from(FILE_ALIGN).expect("image size"),
        );
        write_u32(
            &mut data,
            opt_off.checked_add(60).expect("size of headers"),
            u32::try_from(FILE_ALIGN).expect("headers size"),
        );
        write_u32(&mut data, opt_off.checked_add(108).expect("data dirs"), 16);

        data
    }

    fn add_section(data: &mut Vec<u8>, name: [u8; 8], content: &[u8]) {
        let nt_off = NT_OFFSET;
        let file_hdr = nt_off.checked_add(4).expect("file header");
        let opt_off = file_hdr.checked_add(20).expect("opt header");
        let dd_off = opt_off.checked_add(112).expect("data dirs");
        let shdr_off = dd_off.checked_add(16 * 8).expect("section headers");

        let count_off = file_hdr.checked_add(2).expect("count field");
        let count_end = count_off.checked_add(2).expect("count end");
        let section_index = {
            let chunk = data
                .get(count_off..count_end)
                .and_then(|slice| <[u8; 2]>::try_from(slice).ok())
                .unwrap_or([0; 2]);
            u16::from_le_bytes(chunk)
        };

        let index_usize = usize::from(section_index);
        let raw_offset = FILE_ALIGN
            .checked_mul(index_usize.checked_add(1).expect("index + 1"))
            .expect("raw offset");
        let raw_size = content.len().next_multiple_of(FILE_ALIGN);

        let needed = raw_offset.checked_add(raw_size).expect("needed size");
        data.resize(data.len().max(needed), 0);

        let content_end = raw_offset.checked_add(content.len()).expect("content end");
        data.get_mut(raw_offset..content_end)
            .expect("section content range")
            .copy_from_slice(content);

        let section_header = shdr_off
            .checked_add(index_usize * 40)
            .expect("section header");
        let content_len = u32::try_from(content.len()).expect("content len");
        let raw_offset_u32 = u32::try_from(raw_offset).expect("raw offset");
        let raw_size_u32 = u32::try_from(raw_size).expect("raw size");

        data.get_mut(section_header..section_header.checked_add(8).expect("name range"))
            .expect("section header name range")
            .copy_from_slice(&name);
        write_u32(
            data,
            section_header.checked_add(8).expect("vs"),
            content_len,
        );
        write_u32(
            data,
            section_header.checked_add(12).expect("va"),
            raw_offset_u32,
        );
        write_u32(
            data,
            section_header.checked_add(16).expect("rs"),
            raw_size_u32,
        );
        write_u32(
            data,
            section_header.checked_add(20).expect("ptr"),
            raw_offset_u32,
        );

        let new_count = section_index.checked_add(1).expect("section count");
        write_u16(
            data,
            file_hdr.checked_add(2).expect("count field"),
            new_count,
        );
        write_u32(
            data,
            opt_off.checked_add(56).expect("size of image field"),
            u32::try_from(raw_offset.checked_add(raw_size).expect("image size"))
                .expect("image size"),
        );
    }

    fn write_u64(buf: &mut [u8], offset: usize, value: u64) {
        write_bytes(buf, offset, &value.to_le_bytes());
    }

    #[test]
    fn parse_too_small() {
        // ARRANGE
        let data = [0_u8; 63];

        // ACT
        let err = Sections::parse(&data).unwrap_err();

        // ASSERT
        assert!(err.to_string().contains("too small"), "{err}");
    }

    #[test]
    fn parse_invalid_pe() {
        // ARRANGE
        let data = [0_u8; 256];

        // ACT
        let err = Sections::parse(&data).unwrap_err();

        // ASSERT
        assert!(err.to_string().contains("invalid PE"), "{err}");
    }

    #[test]
    fn parse_missing_kernel_section() {
        // ARRANGE
        let mut data = build_test_pe();
        add_section(&mut data, *b".text\0\0\0", b"placeholder");

        // ACT
        let err = Sections::parse(&data).unwrap_err();

        // ASSERT
        assert!(err.to_string().contains(KERNEL), "{err}");
    }

    #[test]
    fn parse_kernel_only() {
        // ARRANGE
        let mut data = build_test_pe();
        add_section(&mut data, *b".kernel\0", b"kernel_data");

        // ACT
        let sections = Sections::parse(&data).expect("parse should succeed");

        // ASSERT
        assert_eq!(sections.kernel, b"kernel_data");
        assert!(sections.initrd.is_none());
        assert!(sections.cmdline.is_none());
    }

    #[test]
    fn parse_standard_sections() {
        // ARRANGE
        let mut data = build_test_pe();
        add_section(&mut data, *b".kernel\0", b"kernel");
        add_section(&mut data, *b".initrd\0", b"initrd");
        add_section(&mut data, *b".cmdline", b"cmdline");

        // ACT
        let sections = Sections::parse(&data).expect("parse should succeed");

        // ASSERT
        assert_eq!(sections.kernel, b"kernel");
        assert_eq!(sections.initrd.expect("initrd"), b"initrd");
        assert_eq!(sections.cmdline.expect("cmdline"), b"cmdline");
    }

    #[test]
    fn parse_unrecognized_section_skipped() {
        // ARRANGE
        let mut data = build_test_pe();
        add_section(&mut data, *b".unknwn\0", b"ignored");
        add_section(&mut data, *b".kernel\0", b"kernel");

        // ACT
        let sections = Sections::parse(&data).expect("parse should succeed");

        // ASSERT
        assert_eq!(sections.kernel, b"kernel");
    }

    #[test]
    fn iter_sections_kernel_only() {
        // ARRANGE
        let sections = Sections {
            kernel: b"kern",
            initrd: None,
            cmdline: None,
        };

        // ACT
        let items: Vec<_> = sections.iter_sections().collect();

        // ASSERT
        assert_eq!(items, vec![(KERNEL, &b"kern"[..])]);
    }

    #[test]
    fn iter_sections_all_present() {
        // ARRANGE
        let sections = Sections {
            kernel: b"kern",
            initrd: Some(b"initrd"),
            cmdline: Some(b"quiet"),
        };

        // ACT
        let items: Vec<_> = sections.iter_sections().collect();

        // ASSERT
        assert_eq!(
            items,
            vec![
                (KERNEL, &b"kern"[..]),
                (CMDLINE, &b"quiet"[..]),
                (INITRD, &b"initrd"[..]),
            ]
        );
    }

    #[test]
    fn iter_sections_canonical_order() {
        // ARRANGE
        let sections = Sections {
            kernel: b"l",
            initrd: Some(b"i"),
            cmdline: Some(b"c"),
        };

        // ACT
        let names: Vec<&str> = sections.iter_sections().map(|(name, _)| name).collect();

        // ASSERT
        assert_eq!(names, vec![KERNEL, CMDLINE, INITRD]);
    }
}
