//! Kernel network device parsing from link messages.

use alloc::string::String;

use rtnetlink::packet_route::link::{LinkAttribute, LinkFlags, LinkInfo, LinkMessage};

/// A kernel network device as reported by rtnetlink.
#[derive(Debug, Clone)]
pub struct Device {
    /// Kernel interface index.
    pub index: u32,
    /// Interface name as reported by the kernel.
    pub name: String,
    /// Hardware address, defaulting to all zeros when absent or malformed.
    pub mac: [u8; 6],
    /// Whether the device carries a virtual link kind.
    pub is_virtual: bool,
    /// Whether the device is administratively up.
    pub admin_up: bool,
    /// Whether the device reports an active carrier.
    pub carrier: bool,
    /// Index of the master (bridge/bond) this device is enslaved to, if any.
    pub master: Option<u32>,
}

impl Device {
    /// Builds a device from a link message, keeping the raw kernel name.
    #[must_use = "dropping it discards the parsed device"]
    pub fn from_message(message: &LinkMessage) -> Self {
        let (name, mac, is_virtual) = parse_attributes(&message.attributes);
        let flags = message.header.flags;
        Self {
            index: message.header.index,
            name,
            mac,
            is_virtual,
            admin_up: flags.contains(LinkFlags::Up),
            carrier: flags.contains(LinkFlags::LowerUp),
            master: extract_master(message),
        }
    }
}

/// Extracts the 6-byte hardware address from a link message, if present.
#[must_use = "callers need the hardware address to report it"]
pub fn extract_mac(link: &LinkMessage) -> Option<[u8; 6]> {
    for attr in link.attributes.iter().cloned() {
        if let LinkAttribute::Address(address) = attr
            && let Ok(mac) = <[u8; 6]>::try_from(address.as_slice())
        {
            return Some(mac);
        }
    }
    None
}

/// Extracts the interface name from a link message, if present.
#[must_use = "callers need the name to identify the link"]
pub fn extract_name(link: &LinkMessage) -> Option<String> {
    for attr in link.attributes.iter().cloned() {
        if let LinkAttribute::IfName(name) = attr {
            return Some(name);
        }
    }
    None
}

/// Extracts the master (bridge/bond) index from a link message, if present.
#[must_use = "callers need the master index to detect enslaved links"]
pub fn extract_master(link: &LinkMessage) -> Option<u32> {
    for attr in &link.attributes {
        if let &LinkAttribute::Controller(master) = attr
            && master != 0
        {
            return Some(master);
        }
    }
    None
}

/// Returns true if the name matches a physical Ethernet naming convention.
#[must_use = "callers must skip links that are not Ethernet"]
pub fn is_ethernet(name: &str) -> bool {
    if name == "lo" || name.starts_with("wlan") || name.starts_with("wlp") {
        return false;
    }
    name.starts_with("eth")
        || name.starts_with("enp")
        || name.starts_with("ens")
        || name.starts_with("eno")
        || name.starts_with("end")
}

fn parse_attributes(attributes: &[LinkAttribute]) -> (String, [u8; 6], bool) {
    let mut name = String::new();
    let mut mac_address = [0_u8; 6];
    let mut is_virtual = false;

    for attr in attributes.iter().cloned() {
        if let LinkAttribute::IfName(if_name) = attr {
            name = if_name;
            continue;
        }

        if let LinkAttribute::Address(address) = attr {
            mac_address = <[u8; 6]>::try_from(address.as_slice()).unwrap_or(mac_address);
            continue;
        }

        if let LinkAttribute::LinkInfo(info) = attr {
            is_virtual = info
                .iter()
                .any(|link_info| matches!(link_info, LinkInfo::Kind(_)));
        }
    }

    (name, mac_address, is_virtual)
}

#[cfg(test)]
mod tests {
    use rtnetlink::packet_route::link::{InfoKind, LinkAttribute, LinkInfo, LinkMessage};

    use super::{Device, extract_mac, extract_master, extract_name, is_ethernet};

    fn link_with_attributes(attributes: Vec<LinkAttribute>) -> LinkMessage {
        let mut link = LinkMessage::default();
        link.attributes = attributes;
        link
    }

    #[test]
    fn extract_mac_returns_six_byte_address() {
        // ARRANGE
        let link = link_with_attributes(vec![LinkAttribute::Address(vec![1, 2, 3, 4, 5, 6])]);

        // ACT
        let mac = extract_mac(&link);

        // ASSERT
        assert_eq!(mac, Some([1, 2, 3, 4, 5, 6]));
    }

    #[test]
    fn extract_mac_ignores_invalid_address_length() {
        // ARRANGE
        let link = link_with_attributes(vec![LinkAttribute::Address(vec![1, 2, 3])]);

        // ACT
        let mac = extract_mac(&link);

        // ASSERT
        assert!(mac.is_none());
    }

    #[test]
    fn extract_master_returns_controller_index() {
        // ARRANGE
        let link = link_with_attributes(vec![LinkAttribute::Controller(7)]);

        // ACT
        let master = extract_master(&link);

        // ASSERT
        assert_eq!(master, Some(7));
    }

    #[test]
    fn extract_master_returns_none_for_zero_controller() {
        // ARRANGE
        let link = link_with_attributes(vec![LinkAttribute::Controller(0)]);

        // ACT
        let master = extract_master(&link);

        // ASSERT
        assert!(master.is_none());
    }

    #[test]
    fn extract_master_returns_none_without_controller() {
        // ARRANGE
        let link = link_with_attributes(vec![LinkAttribute::IfName("eth0".to_owned())]);

        // ACT
        let master = extract_master(&link);

        // ASSERT
        assert!(master.is_none());
    }

    #[test]
    fn extract_name_returns_interface_name() {
        // ARRANGE
        let link = link_with_attributes(vec![LinkAttribute::IfName("eth0".to_owned())]);

        // ACT
        let name = extract_name(&link);

        // ASSERT
        assert_eq!(name.as_deref(), Some("eth0"));
    }

    #[test]
    fn extract_name_returns_none_when_missing() {
        // ARRANGE
        let link = link_with_attributes(vec![LinkAttribute::Address(vec![1, 2, 3, 4, 5, 6])]);

        // ACT
        let name = extract_name(&link);

        // ASSERT
        assert!(name.is_none());
    }

    #[test]
    fn device_from_message_extracts_name_mac_and_virtual_flag() {
        // ARRANGE
        let link = link_with_attributes(vec![
            LinkAttribute::IfName("eth0".to_owned()),
            LinkAttribute::Address(vec![1, 2, 3, 4, 5, 6]),
            LinkAttribute::LinkInfo(vec![LinkInfo::Kind(InfoKind::Veth)]),
        ]);

        // ACT
        let device = Device::from_message(&link);

        // ASSERT
        assert_eq!(device.name, "eth0");
        assert_eq!(device.mac, [1, 2, 3, 4, 5, 6]);
        assert!(device.is_virtual);
    }

    #[test]
    fn device_from_message_ignores_invalid_mac_length() {
        // ARRANGE
        let link = link_with_attributes(vec![
            LinkAttribute::IfName("eth0".to_owned()),
            LinkAttribute::Address(vec![1, 2, 3]),
        ]);

        // ACT
        let device = Device::from_message(&link);

        // ASSERT
        assert_eq!(device.name, "eth0");
        assert_eq!(device.mac, [0; 6]);
        assert!(!device.is_virtual);
    }

    #[test]
    fn ethernet_interface_name_detection() {
        // ACT / ASSERT
        assert!(is_ethernet("eth0"));
        assert!(is_ethernet("enp3s0"));
        assert!(!is_ethernet("lo"));
        assert!(!is_ethernet("wlan0"));
        assert!(!is_ethernet("br0"));
    }
}
