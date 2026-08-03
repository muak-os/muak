//! `DHCPv4` lease types, state machine, and expiry calculation.

use core::fmt;
use core::net::Ipv4Addr;
use core::time::Duration;
use std::time::SystemTime;

use crate::statemachine::StateMachine;

pub mod client;
pub(crate) mod codec;
pub(crate) mod framing;
pub mod manager;
pub(crate) mod packet;

/// RFC 2131 DHCP client state machine phases.
#[derive(Debug, Clone, PartialEq)]
pub enum State {
    Init,
    Bound,
    Renewing,
    Rebinding,
}

/// A successfully acquired `DHCPv4` lease with timing and server metadata.
#[derive(Debug, Clone)]
pub struct Lease {
    pub obtained_at: SystemTime,
    pub lease_time: Duration,
    pub renewal_time: Duration,
    pub rebind_time: Duration,
    pub server_ip: Ipv4Addr,
    pub assigned_ip: Ipv4Addr,
    pub prefix_len: u8,
    pub gateway: Option<Ipv4Addr>,
    pub dns_servers: Vec<Ipv4Addr>,
}

impl StateMachine for State {
    fn valid_next_states(&self) -> &'static [Self] {
        match *self {
            Self::Init => &[Self::Bound],
            Self::Bound => &[Self::Renewing, Self::Init],
            Self::Renewing => &[Self::Bound, Self::Rebinding, Self::Init],
            Self::Rebinding => &[Self::Bound, Self::Init],
        }
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Init => f.write_str("Init"),
            Self::Bound => f.write_str("Bound"),
            Self::Renewing => f.write_str("Renewing"),
            Self::Rebinding => f.write_str("Rebinding"),
        }
    }
}

impl Lease {
    /// Returns the absolute expiry time of this lease.
    #[must_use]
    pub fn expiry(&self) -> SystemTime {
        self.obtained_at
            .checked_add(self.lease_time)
            .unwrap_or(self.obtained_at)
    }
}

#[cfg(test)]
mod tests {
    use core::time::Duration;
    use std::time::SystemTime;

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
    fn dhcp_lease_expiry_is_obtained_plus_lease_time() {
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
    fn dhcp_lease_expiry_at_epoch() {
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
    fn dhcp_state_display() {
        // ACT / ASSERT
        assert_eq!(State::Init.to_string(), "Init");
        assert_eq!(State::Bound.to_string(), "Bound");
        assert_eq!(State::Renewing.to_string(), "Renewing");
        assert_eq!(State::Rebinding.to_string(), "Rebinding");
    }

    #[test]
    fn valid_dhcp_transitions_succeed() {
        // ARRANGE
        let pairs = [
            (State::Init, State::Bound),
            (State::Bound, State::Renewing),
            (State::Bound, State::Init),
            (State::Renewing, State::Bound),
            (State::Renewing, State::Rebinding),
            (State::Renewing, State::Init),
            (State::Rebinding, State::Bound),
            (State::Rebinding, State::Init),
        ];

        for (from, to) in pairs {
            // ARRANGE
            let mut state = from.clone();

            // ACT
            let result = state.transition(to.clone());

            // ASSERT
            assert!(result.is_ok(), "{from} -> {to} should be valid");
            assert_eq!(state, to);
        }
    }

    #[test]
    fn invalid_dhcp_transitions_return_error() {
        // ARRANGE
        let pairs = [
            (State::Init, State::Renewing),
            (State::Init, State::Rebinding),
            (State::Bound, State::Rebinding),
            (State::Rebinding, State::Renewing),
        ];

        for (from, to) in pairs {
            // ARRANGE
            let mut state = from.clone();

            // ACT
            let result = state.transition(to.clone());

            // ASSERT
            assert!(result.is_err(), "{from} -> {to} should be invalid");
            assert_eq!(state, from, "state must not change on invalid transition");
        }
    }
}
