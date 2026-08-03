//! IPv6 stateless address autoconfiguration for networkd.

pub mod manager;
mod state;

use std::os::fd::{AsFd, AsRawFd as _};

use anyhow::{Result, bail};
pub(crate) use netlib::slaac::icmpv6::ICMPV6_ROUTER_ADVERTISEMENT;

/// `ICMPv6` router-advertisement type for the raw socket filter.
#[repr(C)]
pub(crate) struct Icmp6Filter {
    pub data: [u32; 8],
}

pub(crate) const ICMP6_FILTER: libc::c_int = 1;

pub(crate) fn create_icmpv6_filter() -> Icmp6Filter {
    let mut filter = Icmp6Filter {
        data: [0xFFFF_FFFF; 8],
    };
    let ra_type = usize::from(ICMPV6_ROUTER_ADVERTISEMENT);
    let word = ra_type >> 5;
    let bit = ra_type & 31;
    if let Some(word_ref) = filter.data.get_mut(word) {
        *word_ref &= !(1 << bit);
    }
    filter
}

pub(crate) fn set_icmpv6_filter<Fd: AsFd>(fd: Fd, filter: &Icmp6Filter) -> Result<()> {
    // SAFETY: We pass valid pointers and sizes for the filter struct, fd is valid
    let ret = unsafe {
        libc::setsockopt(
            fd.as_fd().as_raw_fd(),
            libc::IPPROTO_ICMPV6,
            ICMP6_FILTER,
            core::ptr::addr_of!(*filter).cast::<libc::c_void>(),
            libc::socklen_t::try_from(core::mem::size_of::<Icmp6Filter>()).unwrap_or(0),
        )
    };
    if ret < 0 {
        bail!("setsockopt ICMP6_FILTER failed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::os::fd::BorrowedFd;

    use super::*;

    #[test]
    fn create_icmpv6_filter_passes_ra() {
        // ARRANGE / ACT
        let filter = create_icmpv6_filter();
        let ra_type = usize::from(ICMPV6_ROUTER_ADVERTISEMENT);
        let word = filter.data.get(ra_type >> 5).copied().unwrap_or(0);
        let bit = word & (1 << (ra_type & 31));

        // ASSERT
        assert_eq!(bit, 0, "RA type bit should be cleared (pass)");
    }

    #[test]
    fn create_icmpv6_filter_blocks_other_types() {
        // ARRANGE / ACT
        let filter = create_icmpv6_filter();

        // ASSERT
        for icmp_type in [0_u8, 1, 128, 129, 133, 135, 136] {
            if icmp_type == ICMPV6_ROUTER_ADVERTISEMENT {
                continue;
            }
            let idx = usize::from(icmp_type);
            let word = filter.data.get(idx >> 5).copied().unwrap_or(0);
            let bit = word & (1 << (idx & 31));
            assert_ne!(bit, 0, "type {icmp_type} should be blocked");
        }
    }

    #[test]
    fn create_icmpv6_filter_initial_all_blocked() {
        // ARRANGE / ACT
        let filter = create_icmpv6_filter();
        let mut pass_count = 0;
        for word_idx in 0..8 {
            for bit_idx in 0..32 {
                let word = filter.data.get(word_idx).copied().unwrap_or(0);
                if word & (1 << bit_idx) == 0 {
                    pass_count += 1;
                }
            }
        }

        // ASSERT
        assert_eq!(pass_count, 1, "only RA type should pass");
    }

    #[test]
    fn set_icmpv6_filter_fails_on_invalid_fd() {
        // ARRANGE
        let filter = create_icmpv6_filter();

        // ACT
        // SAFETY: fd 9999 is virtually guaranteed to be invalid (EBADF)
        let result = unsafe {
            let bad_fd = BorrowedFd::borrow_raw(9999);
            set_icmpv6_filter(bad_fd, &filter)
        };

        // ASSERT
        result.unwrap_err();
    }
}
