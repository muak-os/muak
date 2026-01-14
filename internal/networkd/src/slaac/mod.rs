mod address;
mod icmpv6;
mod manager;
mod state;

use std::net::Ipv6Addr;

use anyhow::{Result, bail};
use nix::libc;

pub(crate) use icmpv6::ICMPV6_ROUTER_ADVERTISEMENT;

pub use manager::{SlaacEvent, SlaacManager};

pub(crate) const ICMP6_FILTER: libc::c_int = 1;

pub(crate) const FALLBACK_DNS: [Ipv6Addr; 2] = [
    Ipv6Addr::new(0x2620, 0x00fe, 0, 0, 0, 0, 0, 0x00fe), // Quad9
    Ipv6Addr::new(0x2620, 0x00fe, 0, 0, 0, 0, 0, 0x0009),
];

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

pub(crate) fn set_icmpv6_filter(fd: i32, filter: &Icmp6Filter) -> Result<()> {
    let ret = unsafe {
        libc::setsockopt(
            fd,
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

pub(crate) fn get_interface_index(name: &str) -> Result<u32> {
    use std::ffi::CString;
    let cname = CString::new(name)?;
    let idx = unsafe { libc::if_nametoindex(cname.as_ptr()) };
    if idx == 0 {
        bail!("interface not found: {}", name);
    }
    Ok(idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fallback_dns_valid() {
        for dns in &FALLBACK_DNS {
            assert!(!dns.is_unspecified());
            assert!(!dns.is_loopback());
        }
    }
}
