use anyhow::{Context, Result, anyhow, bail};
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
        if data.len() < 0x40 {
            bail!("PE file too small (minimum 64 bytes required)");
        }
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
            validate_section_data(va, vs, data)?;
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

/// Validates section data alignment and sanity checks.
fn validate_section_data(va: usize, vs: usize, data: &[u8]) -> Result<()> {
    if va + vs > data.len() {
        bail!(
            "section data out of bounds: va={} vs={} data_len={}",
            va,
            vs,
            data.len()
        );
    }

    if va % 4096 != 0 && va != 0 {
        bail!("section virtual address not page-aligned: {}", va);
    }

    if vs == 0 {
        bail!("section has zero virtual size");
    }
    if vs > 100 * 1024 * 1024 {
        bail!("section virtual size unreasonably large: {} bytes", vs);
    }
    Ok(())
}
