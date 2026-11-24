//! Failures the `DHCPv4` method reports.

use core::fmt;

// Failure a `DHCPv4` exchange or its socket reported.
#[derive(Debug, Clone, Copy)]
pub(super) enum Fault {
    // The server refused the request, so the client must start over.
    Nak,
    // No response arrived before the exchange deadline.
    Timeout,
    // The offer carried no server identifier option.
    MissingServerId,
    // The response carried no usable address.
    MissingAddress,
    // The message does not fit the method buffers.
    Encoding,
    // The socket backing the exchange failed.
    Socket,
}

impl Fault {
    // Returns true when the failure was a NAK.
    pub(super) const fn is_nak(self) -> bool {
        matches!(self, Self::Nak)
    }
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Nak => f.write_str("DHCP server sent NAK"),
            Self::Timeout => f.write_str("DHCP exchange timed out"),
            Self::MissingServerId => f.write_str("no server identifier in DHCPOFFER"),
            Self::MissingAddress => f.write_str("DHCP response carried no usable address"),
            Self::Encoding => f.write_str("DHCP message does not fit the method buffers"),
            Self::Socket => f.write_str("DHCP socket failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Fault;

    #[test]
    fn nak_is_recognised() {
        // ARRANGE
        let fault = Fault::Nak;
        let timeout = Fault::Timeout;

        // ACT / ASSERT
        assert!(fault.is_nak());
        assert!(!timeout.is_nak());
    }

    #[test]
    fn messages_name_the_failure() {
        // ACT
        let message = Fault::MissingAddress.to_string();

        // ASSERT
        assert_eq!(message, "DHCP response carried no usable address");
    }
}
