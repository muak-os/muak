use anyhow::{Context, Result, anyhow};
use object::LittleEndian as LE;
use object::read::pe::PeFile64;

/// Parsed UKI sections from the PE image.
pub struct UkiSections<'a> {
    pub kernel: Option<&'a [u8]>,
    pub initrd: Option<&'a [u8]>,
    pub cmdline: Option<&'a [u8]>,
}

impl<'a> UkiSections<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        let pe = PeFile64::parse(data).context("Failed to parse PE file")?;

        let sections = pe.section_table();

        let mut result = UkiSections {
            kernel: None,
            initrd: None,
            cmdline: None,
        };

        for section in sections.iter() {
            let name = std::str::from_utf8(&section.name)
                .context("Invalid section name")?
                .trim_end_matches('\0');

            let va = section.virtual_address.get(LE) as usize;
            let vs = section.virtual_size.get(LE) as usize;
            if va + vs > data.len() {
                return Err(anyhow!("section data out of bounds"));
            }
            let section_data = &data[va..va + vs];

            match name {
                ".linux" => result.kernel = Some(section_data),
                ".initrd" => result.initrd = Some(section_data),
                ".cmdline" => result.cmdline = Some(section_data),
                _ => {}
            }
        }

        Ok(result)
    }

    pub fn require_kernel(&self) -> Result<&'a [u8]> {
        self.kernel
            .ok_or_else(|| anyhow!("no .linux section found"))
    }
}
