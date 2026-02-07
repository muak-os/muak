use anyhow::{Context, Result, anyhow, bail};
use object::LittleEndian as LE;
use object::read::pe::PeFile64;

/// Parsed UKI sections from the PE image
pub struct UkiSections<'a> {
    pub kernel: Option<&'a [u8]>,
    pub initrd: Option<&'a [u8]>,
    pub cmdline: Option<&'a [u8]>,
    pub dtb: Option<&'a [u8]>,
}

impl<'a> UkiSections<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        if data.len() < 0x40 {
            bail!("PE file too small (minimum 64 bytes required)");
        }
        let pe = PeFile64::parse(data).context("Failed to parse PE file")?;

        let sections = pe.section_table();

        let mut result = UkiSections {
            kernel: None,
            initrd: None,
            cmdline: None,
            dtb: None,
        };

        for section in sections.iter() {
            let name = std::str::from_utf8(&section.name)
                .context("Invalid section name")?
                .trim_end_matches('\0');

            let rva = section.virtual_address.get(LE) as usize;
            let vs = section.virtual_size.get(LE) as usize;

            if vs == 0 {
                continue;
            }

            if rva + vs > data.len() {
                bail!(
                    "section {} data out of bounds: rva={:#x} size={:#x} data_len={:#x}",
                    name,
                    rva,
                    vs,
                    data.len()
                );
            }

            let section_data = &data[rva..rva + vs];

            match name {
                ".linux" => result.kernel = Some(section_data),
                ".initrd" => result.initrd = Some(section_data),
                ".cmdline" => result.cmdline = Some(section_data),
                ".dtb" => result.dtb = Some(section_data),
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
