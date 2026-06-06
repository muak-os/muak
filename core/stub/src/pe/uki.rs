//! UKI section parsing from PE images.

use core::str;

use anyhow::{Context as _, Result, anyhow, bail};
use object::LittleEndian as LE;
use object::pe::ImageSectionHeader;
use object::read::pe::PeFile64;

/// Parsed UKI sections from the PE image.
#[derive(Debug)]
pub struct Sections<'a> {
    pub linux: &'a [u8],
    pub initrd: Option<&'a [u8]>,
    pub cmdline: Option<&'a [u8]>,
    pub dtb: Option<&'a [u8]>,
}

impl<'a> Sections<'a> {
    /// Parses UKI sections from a PE image.
    ///
    /// # Errors
    ///
    /// Returns an error if the image is not a valid PE file, a section is malformed, or the
    /// required `.linux` section is missing.
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        if data.len() < 0x40 {
            bail!("PE file too small (minimum 64 bytes required)");
        }
        let pe = PeFile64::parse(data).context("Failed to parse PE file")?;

        let mut linux: Option<&'a [u8]> = None;
        let mut initrd: Option<&'a [u8]> = None;
        let mut cmdline: Option<&'a [u8]> = None;
        let mut dtb: Option<&'a [u8]> = None;

        for section in pe
            .section_table()
            .iter()
            .map(|section| uki_section_data(data, section))
            .filter_map(Result::transpose)
        {
            let (name, section_data) = section?;
            set_uki_section(
                name,
                section_data,
                &mut linux,
                &mut initrd,
                &mut cmdline,
                &mut dtb,
            )?;
        }

        Ok(Sections {
            linux: linux.ok_or_else(|| anyhow!("UKI missing required .linux section"))?,
            initrd,
            cmdline,
            dtb,
        })
    }

    /// Returns an iterator over sections to measure, in spec canonical order.
    pub fn iter_sections(&self) -> impl Iterator<Item = (&'static str, &'a [u8])> {
        [
            (".linux", Some(self.linux)),
            (".cmdline", self.cmdline),
            (".initrd", self.initrd),
            (".dtb", self.dtb),
        ]
        .into_iter()
        .filter_map(|(name, data)| data.map(|section_data| (name, section_data)))
    }
}

fn set_uki_section<'a>(
    name: &'static str,
    section_data: &'a [u8],
    linux: &mut Option<&'a [u8]>,
    initrd: &mut Option<&'a [u8]>,
    cmdline: &mut Option<&'a [u8]>,
    dtb: &mut Option<&'a [u8]>,
) -> Result<()> {
    match name {
        ".linux" => *linux = Some(section_data),
        ".initrd" => *initrd = Some(section_data),
        ".cmdline" => *cmdline = Some(section_data),
        ".dtb" => *dtb = Some(section_data),
        unknown => bail!("unexpected UKI section {unknown}"),
    }

    Ok(())
}

fn uki_section_data<'a>(
    data: &'a [u8],
    section: &ImageSectionHeader,
) -> Result<Option<(&'static str, &'a [u8])>> {
    let name = str::from_utf8(&section.name)
        .context("Invalid section name")?
        .trim_end_matches('\0');
    let Some(name) = canonical_uki_section_name(name) else {
        return Ok(None);
    };

    let size =
        usize::try_from(section.virtual_size.get(LE)).context("section size exceeds usize")?;
    if size == 0 {
        return Ok(None);
    }

    let rva =
        usize::try_from(section.virtual_address.get(LE)).context("section RVA exceeds usize")?;
    let end = rva
        .checked_add(size)
        .context("section bounds overflow usize")?;
    let section_data = data.get(rva..end).ok_or_else(|| {
        anyhow!(
            "section {name} data out of bounds: rva={rva:#x} size={size:#x} \
             data_len={:#x}",
            data.len()
        )
    })?;

    Ok(Some((name, section_data)))
}

fn canonical_uki_section_name(name: &str) -> Option<&'static str> {
    match name {
        ".linux" => Some(".linux"),
        ".initrd" => Some(".initrd"),
        ".cmdline" => Some(".cmdline"),
        ".dtb" => Some(".dtb"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pe::fixtures::Builder;

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
        assert!(err.to_string().contains("Failed to parse PE file"), "{err}");
    }

    #[test]
    fn parse_missing_linux_section() {
        // ARRANGE
        let mut builder = Builder::new();
        builder.add_section(*b".text\0\0\0", b"placeholder");
        let data = builder.build();

        // ACT
        let err = Sections::parse(&data).unwrap_err();

        // ASSERT
        assert!(err.to_string().contains("missing required .linux"), "{err}");
    }

    #[test]
    fn parse_linux_only() {
        // ARRANGE
        let mut builder = Builder::new();
        builder.add_section(*b".linux\0\0", b"kernel_data");
        let data = builder.build();

        // ACT
        let sections = Sections::parse(&data).expect("parse should succeed");

        // ASSERT
        assert_eq!(sections.linux, b"kernel_data");
        assert!(sections.initrd.is_none());
        assert!(sections.cmdline.is_none());
        assert!(sections.dtb.is_none());
    }

    #[test]
    fn parse_all_sections() {
        // ARRANGE
        let mut builder = Builder::new();
        builder.add_section(*b".linux\0\0", b"linux");
        builder.add_section(*b".initrd\0", b"initrd");
        builder.add_section(*b".cmdline", b"cmdline");
        builder.add_section(*b".dtb\0\0\0\0", b"dtb");
        let data = builder.build();

        // ACT
        let sections = Sections::parse(&data).expect("parse should succeed");

        // ASSERT
        assert_eq!(sections.linux, b"linux");
        assert_eq!(sections.initrd.expect("initrd"), b"initrd");
        assert_eq!(sections.cmdline.expect("cmdline"), b"cmdline");
        assert_eq!(sections.dtb.expect("dtb"), b"dtb");
    }

    #[test]
    fn parse_unrecognized_section_skipped() {
        // ARRANGE
        let mut builder = Builder::new();
        builder.add_section(*b".unknwn\0", b"ignored");
        builder.add_section(*b".linux\0\0", b"kernel");
        let data = builder.build();

        // ACT
        let sections = Sections::parse(&data).expect("parse should succeed");

        // ASSERT
        assert_eq!(sections.linux, b"kernel");
    }

    #[test]
    fn parse_zero_virtual_size_skipped() {
        // ARRANGE
        let mut builder = Builder::new();
        builder.add_section(*b".linux\0\0", b"kernel");
        builder.add_section(*b".initrd\0", b"initrd_data");
        builder.set_last_virtual_size(0);
        let data = builder.build();

        // ACT
        let sections = Sections::parse(&data).expect("parse should succeed");

        // ASSERT
        assert!(
            sections.initrd.is_none(),
            "zero-size section should be skipped"
        );
    }

    #[test]
    fn parse_section_out_of_bounds() {
        // ARRANGE
        let mut builder = Builder::new();
        builder.add_section(*b".linux\0\0", b"kernel");
        builder.set_last_virtual_size(0xFFFF_FF00);
        let data = builder.build();

        // ACT
        let err = Sections::parse(&data).unwrap_err();

        // ASSERT
        assert!(err.to_string().contains("out of bounds"), "{err}");
    }

    #[test]
    fn iter_sections_linux_only() {
        // ARRANGE
        let sections = Sections {
            linux: b"kern",
            initrd: None,
            cmdline: None,
            dtb: None,
        };

        // ACT
        let items: Vec<_> = sections.iter_sections().collect();

        // ASSERT
        assert_eq!(items, vec![(".linux", &b"kern"[..])]);
    }

    #[test]
    fn iter_sections_all_present() {
        // ARRANGE
        let sections = Sections {
            linux: b"kern",
            initrd: Some(b"initrd"),
            cmdline: Some(b"quiet"),
            dtb: Some(b"dtb"),
        };

        // ACT
        let items: Vec<_> = sections.iter_sections().collect();

        // ASSERT
        assert_eq!(
            items,
            vec![
                (".linux", &b"kern"[..]),
                (".cmdline", &b"quiet"[..]),
                (".initrd", &b"initrd"[..]),
                (".dtb", &b"dtb"[..]),
            ]
        );
    }

    #[test]
    fn iter_sections_canonical_order() {
        // ARRANGE
        let sections = Sections {
            linux: b"l",
            initrd: Some(b"i"),
            cmdline: Some(b"c"),
            dtb: None,
        };

        // ACT
        let names: Vec<&str> = sections.iter_sections().map(|(name, _)| name).collect();

        // ASSERT
        assert_eq!(names, vec![".linux", ".cmdline", ".initrd"]);
    }
}
