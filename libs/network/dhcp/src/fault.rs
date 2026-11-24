//! Failures the `DHCPv4` method reports.

use core::fmt;

#[derive(Debug, Clone, Copy)]
pub(super) enum Fault {
    Nak,
    Timeout,
    MissingServerId,
    MissingAddress,
    Encoding,
    Socket,
}

impl Fault {
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
