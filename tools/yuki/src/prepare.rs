//! Prepares a UKI manifest from a probed stub and component sizes.

use uki::section::{CMDLINE, INITRD, KERNEL, OSREL, canonical_rank};

use crate::error::{Result, YukiError};
use crate::layout::{self, Layout};
use crate::pe::{header, section};
use crate::probe::Probe;

/// A prepared UKI containing everything needed to emit the image.
pub struct Manifest {
    layout: Layout,
    assembly: Assembly,
}

impl Manifest {
    /// Layout of the output UKI.
    #[must_use]
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Byte-assembly instructions for the write pass.
    pub(crate) fn assembly(&self) -> &Assembly {
        &self.assembly
    }
}

/// Byte-assembly instructions for the write pass.
pub(crate) struct Assembly {
    /// Patched header, written first by `write`.
    pub(crate) patched_prefix: Vec<u8>,
    /// Stub bytes after the probed header: `stub_size - probe.consumed`.
    pub(crate) stub_remainder: u64,
    /// File alignment used for zero-padding sections.
    pub(crate) file_alignment: u32,
    /// Sections in write order with zeroed checksums.
    pub(crate) sections: Vec<section::Section>,
}

/// Sizes of the UKI components appended to the stub in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Components {
    /// Size of the `.cmdline` payload.
    pub cmdline: u64,
    /// Size of the `.kernel` payload.
    pub kernel: u64,
    /// Size of the `.initrd` payload.
    pub initramfs: u64,
    /// Size of the `.osrel` payload; zero omits the section.
    pub osrel: u64,
}

/// Computes the full UKI plan from a probed stub and component sizes.
///
/// # Errors
///
/// Returns an error when the stub is truncated (its last section ends past
/// `stub_size`), the stub size is smaller than the probed header, component
/// lengths overflow PE limits, or the section table lacks capacity.
pub fn prepare(probe: Probe, stub_size: u64, components: Components) -> Result<Manifest> {
    let consumed = probe.consumed();
    let stub_remainder = stub_size.checked_sub(consumed).ok_or_else(|| {
        YukiError::InvalidPeStructure(format!(
            "stub size {stub_size} smaller than probed header {consumed}"
        ))
    })?;

    if u64::from(probe.metadata.last_section_file_end) > stub_size {
        return Err(YukiError::InvalidPeStructure(format!(
            "stub truncated: last section ends at {}, stub size {stub_size}",
            probe.metadata.last_section_file_end
        )));
    }

    let mut sizes = [
        (CMDLINE, Some(components.cmdline)),
        (KERNEL, Some(components.kernel)),
        (INITRD, Some(components.initramfs)),
        (OSREL, (components.osrel > 0).then_some(components.osrel)),
    ];
    sizes.sort_by_key(|&(name, _size)| canonical_rank(name));

    let table = section::build_table(&probe.metadata, stub_size, &sizes)?;

    let mut patched_prefix = probe.prefix;
    header::patch(&mut patched_prefix, &probe.metadata, &table)?;

    let layout = layout::from_table(stub_size, &table)?;
    let assembly = Assembly {
        patched_prefix,
        stub_remainder,
        file_alignment: table.file_alignment,
        sections: table.sections,
    };

    Ok(Manifest { layout, assembly })
}

#[cfg(test)]
mod tests {
    use uki::align;
    use uki::metadata::Metadata;
    use uki::section::{CMDLINE, INITRD, KERNEL, OSREL};

    use super::*;

    fn test_metadata() -> Metadata {
        Metadata {
            file_header_offset: 64,
            optional_header_offset: 84,
            section_table_offset: 324,
            size_of_headers: 1024,
            section_alignment: 4096,
            file_alignment: 512,
            last_section_file_end: 512,
            last_section_virtual_end: 4096,
            existing_section_count: 1,
            num_data_directories: 16,
        }
    }

    fn probe_with(metadata: Metadata) -> Probe {
        let prefix = vec![0_u8; usize::try_from(metadata.size_of_headers).unwrap()];
        Probe {
            consumed: u64::try_from(prefix.len()).unwrap(),
            prefix,
            metadata,
        }
    }

    #[test]
    fn prepare_rejects_stub_size_smaller_than_consumed() {
        // ARRANGE
        let probe = probe_with(test_metadata());

        // ACT
        let result = prepare(
            probe,
            100,
            Components {
                cmdline: 10,
                kernel: 100,
                initramfs: 100,
                osrel: 0,
            },
        );

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(msg))
                if msg.contains("smaller than probed header")
        ));
    }

    #[test]
    fn prepare_orders_offsets() {
        // ARRANGE
        let probe = probe_with(test_metadata());

        // ACT
        let manifest = prepare(
            probe,
            1024,
            Components {
                cmdline: 10,
                kernel: 2048,
                initramfs: 4096,
                osrel: 0,
            },
        )
        .unwrap();

        // ASSERT
        let layout = manifest.layout();
        assert!(layout.cmdline_offset < layout.initramfs_offset);
        assert!(layout.initramfs_offset < layout.kernel_offset);
        assert!(layout.kernel_offset < layout.total_size);
    }

    #[test]
    fn prepare_places_osrel_after_kernel() {
        // ARRANGE
        let probe = probe_with(test_metadata());

        // ACT
        let manifest = prepare(
            probe,
            1024,
            Components {
                cmdline: 10,
                kernel: 2048,
                initramfs: 4096,
                osrel: 64,
            },
        )
        .unwrap();

        // ASSERT
        let layout = manifest.layout();
        assert!(layout.kernel_offset < layout.osrel_offset);
        assert!(layout.osrel_offset < layout.total_size);
        let osrel_is_last = manifest
            .assembly()
            .sections
            .last()
            .is_some_and(|section| section.name == OSREL);
        assert!(osrel_is_last, "osrel must be the last planned section");
    }

    #[test]
    fn prepare_omits_osrel_when_zero() {
        // ARRANGE
        let probe = probe_with(test_metadata());

        // ACT
        let manifest = prepare(
            probe,
            1024,
            Components {
                cmdline: 10,
                kernel: 2048,
                initramfs: 4096,
                osrel: 0,
            },
        )
        .unwrap();

        // ASSERT
        assert!(
            !manifest
                .assembly()
                .sections
                .iter()
                .any(|section| section.name == OSREL)
        );
        assert_eq!(manifest.layout().osrel_offset, 0);
    }

    #[test]
    fn prepare_layout_matches_assembly_sections() {
        // ARRANGE
        let probe = probe_with(test_metadata());

        // ACT
        let manifest = prepare(
            probe,
            1024,
            Components {
                cmdline: 10,
                kernel: 2048,
                initramfs: 4096,
                osrel: 0,
            },
        )
        .unwrap();

        // ASSERT
        for planned in &manifest.assembly().sections {
            let expected = match planned.name {
                CMDLINE => manifest.layout().cmdline_offset,
                KERNEL => manifest.layout().kernel_offset,
                INITRD => manifest.layout().initramfs_offset,
                OSREL => manifest.layout().osrel_offset,
                _ => panic!("unexpected section '{}'", planned.name),
            };
            assert_eq!(
                u64::try_from(planned.file_offset).unwrap(),
                expected,
                "layout offset for '{}' should match the assembly section",
                planned.name
            );
        }
    }

    #[test]
    fn prepare_total_size_matches_last_aligned_section_end() {
        // ARRANGE
        let probe = probe_with(test_metadata());

        // ACT
        let manifest = prepare(
            probe,
            1024,
            Components {
                cmdline: 10,
                kernel: 2048,
                initramfs: 4096,
                osrel: 0,
            },
        )
        .unwrap();

        // ASSERT
        let last = manifest.assembly().sections.last().unwrap();
        let aligned = align::to(
            u32::try_from(last.size).unwrap(),
            manifest.assembly().file_alignment,
        );
        let last_end = last
            .file_offset
            .saturating_add(usize::try_from(aligned).unwrap());
        assert_eq!(
            manifest.layout().total_size,
            u64::try_from(last_end).unwrap(),
            "total size should match the last aligned section end"
        );
    }

    #[test]
    fn prepare_rejects_too_many_sections() {
        // ARRANGE
        let metadata = Metadata {
            existing_section_count: u16::MAX.saturating_sub(2),
            ..test_metadata()
        };
        let probe = probe_with(metadata);

        // ACT
        let result = prepare(
            probe,
            1024,
            Components {
                cmdline: 10,
                kernel: 100,
                initramfs: 100,
                osrel: 0,
            },
        );

        // ASSERT
        assert!(matches!(result, Err(YukiError::TooManySections)));
    }

    #[test]
    fn prepare_rejects_insufficient_header_capacity() {
        // ARRANGE
        let metadata = Metadata {
            size_of_headers: 368,
            ..test_metadata()
        };
        let probe = probe_with(metadata);

        // ACT
        let result = prepare(
            probe,
            1024,
            Components {
                cmdline: 10,
                kernel: 100,
                initramfs: 100,
                osrel: 0,
            },
        );

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(msg))
                if msg.contains("section table exceeds size of headers")
        ));
    }

    #[test]
    fn prepare_rejects_truncated_stub() {
        // ARRANGE
        let metadata = Metadata {
            size_of_headers: 512,
            last_section_file_end: 4096,
            ..test_metadata()
        };
        let probe = probe_with(metadata);

        // ACT
        let result = prepare(
            probe,
            2048,
            Components {
                cmdline: 10,
                kernel: 100,
                initramfs: 100,
                osrel: 0,
            },
        );

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(msg))
                if msg.contains("stub truncated")
        ));
    }
}
