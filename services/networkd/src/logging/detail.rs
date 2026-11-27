//! Address family details rendered on interface observation lines.

use core::fmt;

use net_observe::interface;

const MAX_DETAIL: usize = 8;

pub(super) fn dhcp(
    formatter: &mut fmt::Formatter<'_>,
    name: &netmodel::Name,
    lease: &interface::DhcpLease,
) -> fmt::Result {
    write!(
        formatter,
        "{name}: IPv4 dhcp {}/{} server {} gateway ",
        lease.address, lease.prefix_len, lease.server
    )?;
    match lease.gateway {
        Some(gateway) => write!(formatter, "{gateway}")?,
        None => formatter.write_str("none")?,
    }
    formatter.write_str(" dns ")?;
    address_list(formatter, &lease.dns)?;

    write!(formatter, " ({})", lease.phase)
}

pub(super) fn slaac(
    formatter: &mut fmt::Formatter<'_>,
    name: &netmodel::Name,
    update: &interface::SlaacUpdate,
) -> fmt::Result {
    write!(formatter, "{name}: IPv6 slaac ")?;
    prefix_list(formatter, &update.addresses)?;
    formatter.write_str(" routers ")?;
    address_list(formatter, &update.routers)?;
    formatter.write_str(" dns ")?;

    address_list(formatter, &update.dns)
}

fn address_list<A: fmt::Display>(
    formatter: &mut fmt::Formatter<'_>,
    addresses: &[A],
) -> fmt::Result {
    formatter.write_str("[")?;
    for (position, address) in addresses.iter().take(MAX_DETAIL).enumerate() {
        if position > 0 {
            formatter.write_str(", ")?;
        }
        write!(formatter, "{address}")?;
    }
    let remaining = addresses.len().saturating_sub(MAX_DETAIL);
    if remaining > 0 {
        write!(formatter, ", +{remaining} more")?;
    }

    formatter.write_str("]")
}

fn prefix_list<A: fmt::Display>(
    formatter: &mut fmt::Formatter<'_>,
    addresses: &[(A, u8)],
) -> fmt::Result {
    formatter.write_str("[")?;
    for (position, entry) in addresses.iter().take(MAX_DETAIL).enumerate() {
        if position > 0 {
            formatter.write_str(", ")?;
        }
        write!(formatter, "{}/{}", entry.0, entry.1)?;
    }
    let remaining = addresses.len().saturating_sub(MAX_DETAIL);
    if remaining > 0 {
        write!(formatter, ", +{remaining} more")?;
    }

    formatter.write_str("]")
}
