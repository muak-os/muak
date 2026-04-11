//! DHCPv4 lease type and expiry calculation.

use std::time::{Duration, SystemTime};

pub mod client;

#[derive(Debug, Clone)]
pub struct DhcpLease {
    pub obtained_at: SystemTime,
    pub lease_time: Duration,
    pub renewal_time: Duration,
    pub rebind_time: Duration,
}

impl DhcpLease {
    /// Returns the absolute expiry time of this lease.
    pub fn expiry(&self) -> SystemTime {
        self.obtained_at + self.lease_time
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::*;

    #[test]
    fn dhcp_lease_expiry_is_obtained_plus_lease_time() {
        // ARRANGE
        let obtained = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let lease = DhcpLease {
            obtained_at: obtained,
            lease_time: Duration::from_secs(3600),
            renewal_time: Duration::from_secs(1800),
            rebind_time: Duration::from_secs(3150),
        };
        let expected = obtained + Duration::from_secs(3600);

        // ACT
        let result = lease.expiry();

        // ASSERT
        assert_eq!(result, expected);
    }

    #[test]
    fn dhcp_lease_expiry_at_epoch() {
        // ARRANGE
        let lease = DhcpLease {
            obtained_at: SystemTime::UNIX_EPOCH,
            lease_time: Duration::from_secs(86400),
            renewal_time: Duration::from_secs(43200),
            rebind_time: Duration::from_secs(75600),
        };
        let expected = SystemTime::UNIX_EPOCH + Duration::from_secs(86400);

        // ACT
        let result = lease.expiry();

        // ASSERT
        assert_eq!(result, expected);
    }
}
