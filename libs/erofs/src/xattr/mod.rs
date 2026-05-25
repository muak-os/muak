//! Inline xattr encoding for security.selinux labels.

mod filter;
mod hash;
mod selinux;

/// Compute the inline xattr payload for a single `security.selinux` label.
pub fn selinux_payload(label: &[u8]) -> Vec<u8> {
    selinux::payload(label)
}

/// Compute `i_xattr_icount` from the total inline xattr payload size.
pub fn icount(payload_size: usize) -> u16 {
    selinux::icount(payload_size)
}
