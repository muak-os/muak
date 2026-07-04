//! Name-filter computation for inline xattr lookups.

pub(super) const XATTR_FILTER_SEED: u32 = 0x25BB_E08F;
const XATTR_FILTER_DEFAULT: u32 = u32::MAX;
const XATTR_FILTER_BITS: u32 = 32;

use super::hash::xxhash32;

/// Compute the xattr name filter bloom-bit for an xattr.
pub(super) fn compute_name_filter(base_index: u8, name_suffix: &[u8]) -> u32 {
    let hash = xxhash32(
        name_suffix,
        XATTR_FILTER_SEED.saturating_add(u32::from(base_index)),
    );
    let bit = hash & XATTR_FILTER_BITS.saturating_sub(1);
    XATTR_FILTER_DEFAULT & !(1_u32 << bit)
}

#[cfg(test)]
mod tests {
    use super::compute_name_filter;
    use crate::xattr::selinux::EROFS_XATTR_INDEX_SECURITY;

    #[test]
    fn compute_name_filter_produces_valid_mask() {
        // ACT
        let filter = compute_name_filter(EROFS_XATTR_INDEX_SECURITY, b"selinux");

        // ASSERT
        assert_ne!(filter, u32::MAX);
        assert_ne!(filter, 0);
    }

    #[test]
    fn compute_name_filter_different_indices() {
        // ACT
        let first = compute_name_filter(EROFS_XATTR_INDEX_SECURITY, b"selinux");
        let second = compute_name_filter(EROFS_XATTR_INDEX_SECURITY + 1, b"selinux");

        // ASSERT
        assert_ne!(first, second);
    }
}
