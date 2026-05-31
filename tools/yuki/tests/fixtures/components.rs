/// Generates a fake Linux kernel image.
#[must_use]
pub fn fake_kernel(size: usize) -> Vec<u8> {
    let mut kernel = Vec::with_capacity(size);
    kernel.extend_from_slice(b"KERNEL_MAGIC");
    kernel.extend(
        (0_u8..=u8::MAX)
            .cycle()
            .take(size.saturating_sub(kernel.len())),
    );
    kernel.truncate(size);
    kernel
}

/// Generates a fake initrd image with gzip magic.
#[must_use]
pub fn fake_initrd(size: usize) -> Vec<u8> {
    let mut initrd = Vec::with_capacity(size);
    initrd.extend_from_slice(&[0x1f, 0x8b, 0x08, 0x00]);
    initrd.resize(size, 0xAA);
    initrd.truncate(size);
    initrd
}

/// Generates a sample kernel command line.
#[must_use]
pub fn sample_cmdline() -> Vec<u8> {
    b"console=ttyS0 quiet".to_vec()
}

/// Generates a fake Device Tree Blob with FDT magic.
#[must_use]
pub fn fake_dtb(size: usize) -> Vec<u8> {
    let mut dtb = Vec::with_capacity(size);
    dtb.extend_from_slice(&[0xd0, 0x0d, 0xfe, 0xed]);
    dtb.extend_from_slice(&[0x00, 0x00, 0x00, 0x11]);
    dtb.resize(size, 0x00);
    dtb.truncate(size);
    dtb
}
