//! Integration tests for UKI section parsing.

#[cfg(test)]
mod common;

#[cfg(test)]
mod tests {
    use uki::section::{CMDLINE, INITRD, KERNEL, Sections};

    use crate::common::{add_section, build_test_pe};

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
