//! IPv6 stateless address autoconfiguration for networkd.

mod manager;
mod state;

use std::os::fd::{AsFd, AsRawFd};

use anyhow::{Result, bail};
pub use manager::{SlaacEvent, SlaacManager};
pub(crate) use netlib::slaac::icmpv6::ICMPV6_ROUTER_ADVERTISEMENT;

pub(crate) const ICMP6_FILTER: libc::c_int = 1;

#[repr(C)]
pub(crate) struct Icmp6Filter {
    pub data: [u32; 8],
}

pub(crate) fn create_icmpv6_filter() -> Icmp6Filter {
    let mut filter = Icmp6Filter {
        data: [0xFFFFFFFF; 8],
    };
    let ra_type = ICMPV6_ROUTER_ADVERTISEMENT as usize;
    filter.data[ra_type / 32] &= !(1 << (ra_type % 32));
    filter
}

pub(crate) fn set_icmpv6_filter<Fd: AsFd>(fd: Fd, filter: &Icmp6Filter) -> Result<()> {
    // SAFETY: We pass valid pointers and sizes for the filter struct, fd is valid
    let ret = unsafe {
        libc::setsockopt(
            fd.as_fd().as_raw_fd(),
            libc::IPPROTO_ICMPV6,
            ICMP6_FILTER,
            filter as *const _ as *const libc::c_void,
            std::mem::size_of::<Icmp6Filter>() as libc::socklen_t,
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
        let ra_type = ICMPV6_ROUTER_ADVERTISEMENT as usize;
        let bit = filter.data[ra_type / 32] & (1 << (ra_type % 32));

        // ASSERT
        assert_eq!(bit, 0, "RA type bit should be cleared (pass)");
    }

    #[test]
    fn create_icmpv6_filter_blocks_other_types() {
        // ARRANGE / ACT
        let filter = create_icmpv6_filter();

        // ASSERT
        for icmp_type in [0u8, 1, 128, 129, 133, 135, 136] {
            if icmp_type == ICMPV6_ROUTER_ADVERTISEMENT {
                continue;
            }
            let idx = icmp_type as usize;
            let bit = filter.data[idx / 32] & (1 << (idx % 32));
            assert_ne!(bit, 0, "type {} should be blocked", icmp_type);
        }
    }

    #[test]
    fn create_icmpv6_filter_initial_all_blocked() {
        // ARRANGE / ACT
        let filter = create_icmpv6_filter();
        let mut pass_count = 0;
        for word_idx in 0..8 {
            for bit_idx in 0..32 {
                if filter.data[word_idx] & (1 << bit_idx) == 0 {
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
        assert!(result.is_err(), "should fail with invalid fd");
    }
}
