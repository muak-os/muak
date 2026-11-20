//! `DHCPv4` lease type and expiry calculation.

use core::net::Ipv4Addr;
use core::time::Duration;
use std::time::SystemTime;

use dhcp_proto::ParsedOptions;

// Lease duration used when the server omits the option.
const DEFAULT_LEASE_SECS: u32 = 3600;
// Prefix length used when the server omits the subnet mask.
const DEFAULT_PREFIX_LEN: u8 = 24;

/// A successfully acquired `DHCPv4` lease with timing and server metadata.
#[derive(Debug, Clone)]
pub struct Lease {
    /// Wall-clock time the lease was obtained at.
    pub obtained_at: SystemTime,
    /// Total lease duration granted by the server.
    pub lease_time: Duration,
    /// Time after acquisition at which to renew.
    pub renewal_time: Duration,
    /// Time after acquisition at which to rebind.
    pub rebind_time: Duration,
    /// Address of the server that granted the lease.
    pub server_ip: Ipv4Addr,
    /// Address assigned to the interface.
    pub assigned_ip: Ipv4Addr,
    /// Prefix length of the assigned subnet.
    pub prefix_len: u8,
    /// Default gateway, when the server advertised one.
    pub gateway: Option<Ipv4Addr>,
    /// DNS servers advertised by the server.
    pub dns_servers: Vec<Ipv4Addr>,
}

impl Lease {
    /// Returns the absolute expiry time of this lease.
    #[must_use = "callers must renew the lease before it expires"]
    pub fn expiry(&self) -> SystemTime {
        self.obtained_at
            .checked_add(self.lease_time)
            .unwrap_or(self.obtained_at)
    }

    /// Builds a lease from a validated ACK response.
    pub(crate) fn from_ack(
        obtained_at: SystemTime,
        assigned_ip: Ipv4Addr,
        server_ip: Ipv4Addr,
        options: ParsedOptions,
    ) -> Self {
        let lease_seconds = options.lease_time.unwrap_or(DEFAULT_LEASE_SECS);
        let prefix_len = options.subnet_mask.map_or(DEFAULT_PREFIX_LEN, |mask| {
            let ones = mask.octets().into_iter().map(u8::count_ones).sum::<u32>();
            u8::try_from(ones).unwrap_or(DEFAULT_PREFIX_LEN)
        });
        let renewal = lease_seconds.saturating_div(2);
        let rebind = lease_seconds.saturating_mul(7).saturating_div(8);

        Self {
            obtained_at,
            lease_time: Duration::from_secs(u64::from(lease_seconds)),
            renewal_time: Duration::from_secs(u64::from(renewal)),
            rebind_time: Duration::from_secs(u64::from(rebind)),
            server_ip,
            assigned_ip,
            prefix_len,
            gateway: options.router,
            dns_servers: options.dns_servers,
        }
    }
}

#[cfg(test)]
mod tests {
    use core::time::Duration;
    use std::time::SystemTime;

    use dhcp_proto::message_type;

    use super::*;

    fn make_lease(obtained_at: SystemTime, lease_secs: u64) -> Lease {
        Lease {
            obtained_at,
            lease_time: Duration::from_secs(lease_secs),
            renewal_time: Duration::from_secs(lease_secs.saturating_div(2)),
            rebind_time: Duration::from_secs(lease_secs.saturating_mul(7).saturating_div(8)),
            server_ip: Ipv4Addr::new(192, 168, 1, 1),
            assigned_ip: Ipv4Addr::new(192, 168, 1, 100),
            prefix_len: 24,
            gateway: Some(Ipv4Addr::new(192, 168, 1, 1)),
            dns_servers: vec![Ipv4Addr::new(8, 8, 8, 8)],
        }
    }

    #[test]
    fn expiry_is_obtained_plus_lease_time() {
        // ARRANGE
        let obtained = SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_secs(1000))
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let lease = make_lease(obtained, 3600);
        let expected = obtained
            .checked_add(Duration::from_hours(1))
            .unwrap_or(obtained);

        // ACT
        let result = lease.expiry();

        // ASSERT
        assert_eq!(result, expected);
    }

    #[test]
    fn expiry_at_epoch() {
        // ARRANGE
        let lease = make_lease(SystemTime::UNIX_EPOCH, 86400);
        let expected = SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_hours(24))
            .unwrap_or(SystemTime::UNIX_EPOCH);

        // ACT
        let result = lease.expiry();

        // ASSERT
        assert_eq!(result, expected);
    }

    #[test]
    fn from_ack_defaults() {
        // ARRANGE
        let options = ParsedOptions {
            message_type: Some(message_type::ACK),
            server_id: Some(Ipv4Addr::new(192, 168, 1, 1)),
            subnet_mask: None,
            router: None,
            dns_servers: Vec::new(),
            lease_time: None,
        };

        // ACT
        let lease = Lease::from_ack(
            SystemTime::UNIX_EPOCH,
            Ipv4Addr::new(10, 0, 0, 1),
            Ipv4Addr::new(192, 168, 1, 1),
            options,
        );

        // ASSERT
        assert_eq!(lease.assigned_ip, Ipv4Addr::new(10, 0, 0, 1));
        assert_eq!(lease.server_ip, Ipv4Addr::new(192, 168, 1, 1));
        assert_eq!(lease.prefix_len, DEFAULT_PREFIX_LEN);
        assert_eq!(
            lease.lease_time,
            Duration::from_secs(u64::from(DEFAULT_LEASE_SECS))
        );
        assert!(lease.gateway.is_none());
        assert_eq!(lease.dns_servers, Vec::<Ipv4Addr>::new());
    }

    #[test]
    fn from_ack_with_all_options() {
        // ARRANGE
        let options = ParsedOptions {
            message_type: Some(message_type::ACK),
            server_id: Some(Ipv4Addr::new(192, 168, 1, 1)),
            subnet_mask: Some(Ipv4Addr::new(255, 255, 255, 0)),
            router: Some(Ipv4Addr::new(192, 168, 1, 1)),
            dns_servers: vec![Ipv4Addr::new(8, 8, 8, 8)],
            lease_time: Some(7200),
        };

        // ACT
        let lease = Lease::from_ack(
            SystemTime::UNIX_EPOCH,
            Ipv4Addr::new(10, 0, 0, 1),
            Ipv4Addr::new(192, 168, 1, 1),
            options,
        );

        // ASSERT
        assert_eq!(lease.prefix_len, 24);
        assert_eq!(lease.gateway, Some(Ipv4Addr::new(192, 168, 1, 1)));
        assert_eq!(lease.dns_servers, vec![Ipv4Addr::new(8, 8, 8, 8)]);
        assert_eq!(lease.lease_time, Duration::from_hours(2));
        assert_eq!(lease.renewal_time, Duration::from_hours(1));
        assert_eq!(lease.rebind_time, Duration::from_mins(105));
    }
}
