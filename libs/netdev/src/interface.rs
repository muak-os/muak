//! Ethernet interface enumeration and priority-based selection.

use core::cmp::Ordering;
use core::future::Future;

use netlink::link::device::{Device, is_ethernet};
use netlink::link::ifname::Name;
use netlink::link::state::State;
use netlink::{Rtnl, link};
use thiserror::Error;

/// Interface enumeration failures.
#[derive(Debug, Error)]
pub enum Failure {
    /// Failed to enumerate links.
    #[error("failed to enumerate links: {0}")]
    List(#[from] link::Failure),
    /// Kernel returned invalid interface name.
    #[error("kernel returned invalid interface name: {0}")]
    InvalidName(#[from] link::ifname::InvalidName),
}

/// Interface enumeration result type.
pub type Result<T> = core::result::Result<T, Failure>;

/// A physical Ethernet interface descriptor.
#[derive(Debug, Clone)]
pub struct Ethernet {
    /// Interface name.
    pub name: Name,
    /// Kernel interface index.
    pub index: u32,
    /// MAC address.
    pub mac_address: [u8; 6],
    /// Current link state.
    pub link_state: State,
}

impl Ethernet {
    /// Constructs a new interface descriptor.
    #[must_use = "dropping it discards the interface descriptor"]
    pub const fn new(name: Name, index: u32, mac_address: [u8; 6], link_state: State) -> Self {
        Self {
            name,
            index,
            mac_address,
            link_state,
        }
    }

    /// Returns true when the underlying link has an active carrier signal.
    #[must_use = "callers must skip links without a carrier"]
    pub fn has_carrier(&self) -> bool {
        self.link_state.has_carrier()
    }
}

/// Trait covering interface enumeration netlink operations.
pub trait Ops: Clone + Send + Sync + 'static {
    /// Lists all Ethernet interfaces on the system.
    fn discover_ethernet(&self) -> impl Future<Output = Result<Vec<Ethernet>>> + Send;
}

impl Ops for Rtnl {
    async fn discover_ethernet(&self) -> Result<Vec<Ethernet>> {
        let mut interfaces = Vec::new();
        for device in link::Ops::devices(self).await? {
            interfaces.extend(to_ethernet(device)?);
        }

        Ok(interfaces)
    }
}

/// Interface selection logic for choosing the best primary and backup interfaces.
pub struct Selector;

impl Selector {
    /// Selects the highest-priority interface as the primary.
    #[must_use = "callers must handle having no usable interface"]
    pub fn select_primary(interfaces: &[Ethernet]) -> Option<&Ethernet> {
        if interfaces.is_empty() {
            return None;
        }

        interfaces
            .iter()
            .max_by(|left, right| Self::compare_interfaces(left, right))
    }

    /// Returns all non-primary interfaces sorted by descending priority.
    #[must_use = "callers need the remaining interfaces to build the bond"]
    pub fn select_backups<'a>(
        interfaces: &'a [Ethernet],
        primary_name: &Name,
    ) -> Vec<&'a Ethernet> {
        let mut backups: Vec<&Ethernet> = interfaces
            .iter()
            .filter(|interface| &interface.name != primary_name)
            .collect();

        backups.sort_by(|left, right| Self::compare_interfaces(left, right).reverse());

        backups
    }

    fn compare_interfaces(left: &Ethernet, right: &Ethernet) -> Ordering {
        let score_left = Self::score_interface(left);
        let score_right = Self::score_interface(right);

        score_left
            .cmp(&score_right)
            .then_with(|| right.index.cmp(&left.index))
    }

    fn score_interface(interface: &Ethernet) -> u32 {
        let mut score = 0_u32;

        if interface.has_carrier() {
            score = score.saturating_add(2000);
        }

        if interface.link_state != State::Down {
            score = score.saturating_add(1000);
        }

        score = score.saturating_add(Self::score_naming(&interface.name));

        score
    }

    fn score_naming(name: &Name) -> u32 {
        let interface_name = name.as_str();
        if interface_name.starts_with("eno") {
            500
        } else if interface_name.starts_with("ens") {
            400
        } else if interface_name.starts_with("enp") {
            300
        } else if interface_name.starts_with("end") {
            200
        } else if interface_name.starts_with("eth") {
            100
        } else {
            50
        }
    }
}

fn to_ethernet(device: Device) -> Result<Option<Ethernet>> {
    if device.name.is_empty() || !is_ethernet(&device.name) || device.is_virtual {
        return Ok(None);
    }

    let name = Name::new(device.name)?;
    let link_state = match (device.admin_up, device.carrier) {
        (true, true) => State::Up,
        (true, false) => State::NoCarrier,
        (false, _) => State::Down,
    };

    Ok(Some(Ethernet::new(
        name,
        device.index,
        device.mac,
        link_state,
    )))
}

#[cfg(test)]
mod tests {
    use netlink::link::state::State as LinkStateKind;

    use super::*;

    type Interface = Ethernet;
    type InterfaceSelector = Selector;

    fn make_interface(name: &str, link_state: LinkStateKind) -> Interface {
        make_interface_with_index(name, 0, link_state)
    }

    fn make_interface_with_index(name: &str, index: u32, link_state: LinkStateKind) -> Interface {
        Interface {
            name: Name::new(name).unwrap(),
            index,
            mac_address: [0, 0, 0, 0, 0, 0],
            link_state,
        }
    }

    fn make_device(name: &str, is_virtual: bool, admin_up: bool, carrier: bool) -> Device {
        Device {
            index: 0,
            name: name.to_owned(),
            mac: [0; 6],
            is_virtual,
            admin_up,
            carrier,
            master: None,
        }
    }

    #[test]
    fn to_ethernet_rejects_non_ethernet_names() {
        // ARRANGE
        let loopback = make_device("lo", false, true, true);
        let wireless = make_device("wlan0", false, true, true);

        // ACT
        let loopback_result = to_ethernet(loopback);
        let wireless_result = to_ethernet(wireless);

        // ASSERT
        assert!(loopback_result.expect("valid device").is_none());
        assert!(wireless_result.expect("valid device").is_none());
    }

    #[test]
    fn to_ethernet_rejects_virtual_device() {
        // ARRANGE
        let device = make_device("eth0", true, true, true);

        // ACT
        let result = to_ethernet(device);

        // ASSERT
        assert!(result.expect("valid device").is_none());
    }

    #[test]
    fn to_ethernet_maps_flag_combinations() {
        // ARRANGE
        let up = make_device("eth0", false, true, true);
        let no_carrier = make_device("eth0", false, true, false);
        let down = make_device("eth0", false, false, true);

        // ACT
        let up_state = to_ethernet(up)
            .expect("valid device")
            .expect("ethernet interface")
            .link_state;
        let no_carrier_state = to_ethernet(no_carrier)
            .expect("valid device")
            .expect("ethernet interface")
            .link_state;
        let down_state = to_ethernet(down)
            .expect("valid device")
            .expect("ethernet interface")
            .link_state;

        // ASSERT
        assert_eq!(up_state, LinkStateKind::Up);
        assert_eq!(no_carrier_state, LinkStateKind::NoCarrier);
        assert_eq!(down_state, LinkStateKind::Down);
    }

    #[test]
    fn to_ethernet_preserves_index_and_mac() {
        // ARRANGE
        let mut device = make_device("eth0", false, true, true);
        device.index = 7;
        device.mac = [1, 2, 3, 4, 5, 6];

        // ACT
        let interface = to_ethernet(device)
            .expect("valid device")
            .expect("ethernet interface");

        // ASSERT
        assert_eq!(interface.name, "eth0");
        assert_eq!(interface.index, 7);
        assert_eq!(interface.mac_address, [1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn ethernet_new_populates_fields() {
        // ARRANGE
        let name = Name::new("eth0").expect("valid name");
        let mac = [1, 2, 3, 4, 5, 6];

        // ACT
        let interface = Interface::new(name.clone(), 7, mac, LinkStateKind::Up);

        // ASSERT
        assert_eq!(interface.name, name);
        assert_eq!(interface.index, 7);
        assert_eq!(interface.mac_address, mac);
        assert_eq!(interface.link_state, LinkStateKind::Up);
    }

    #[test]
    fn select_primary_prefers_carrier() {
        // ARRANGE
        let interfaces = vec![
            make_interface("eth0", LinkStateKind::NoCarrier),
            make_interface("eth1", LinkStateKind::Up),
        ];

        // ACT
        let primary = InterfaceSelector::select_primary(&interfaces);

        // ASSERT
        assert_eq!(primary.expect("should have primary").name, "eth1");
    }

    #[test]
    fn select_primary_prefers_no_carrier_over_down() {
        // ARRANGE
        let interfaces = vec![
            make_interface("eth0", LinkStateKind::Down),
            make_interface("eth1", LinkStateKind::NoCarrier),
        ];

        // ACT
        let primary = InterfaceSelector::select_primary(&interfaces);

        // ASSERT
        assert_eq!(primary.expect("should have primary").name, "eth1");
    }

    #[test]
    fn select_primary_prefers_better_naming() {
        // ARRANGE
        let interfaces = vec![
            make_interface("eth0", LinkStateKind::Up),
            make_interface("eno1", LinkStateKind::Up),
        ];

        // ACT
        let primary = InterfaceSelector::select_primary(&interfaces);

        // ASSERT
        assert_eq!(primary.expect("should have primary").name, "eno1");
    }

    #[test]
    fn naming_priority_order() {
        // ARRANGE
        let interfaces = vec![
            make_interface("eth0", LinkStateKind::Up),
            make_interface("enp3s0", LinkStateKind::Up),
            make_interface("ens1", LinkStateKind::Up),
            make_interface("eno1", LinkStateKind::Up),
        ];

        // ACT
        let primary = InterfaceSelector::select_primary(&interfaces);

        // ASSERT
        assert_eq!(primary.expect("should have primary").name, "eno1");
    }

    #[test]
    fn select_primary_scores_end_above_eth() {
        // ARRANGE
        let interfaces = vec![
            make_interface("eth0", LinkStateKind::Up),
            make_interface("end0", LinkStateKind::Up),
        ];

        // ACT
        let primary = InterfaceSelector::select_primary(&interfaces);

        // ASSERT
        assert_eq!(primary.expect("should have primary").name, "end0");
    }

    #[test]
    fn select_primary_handles_unknown_ethernet_like_name() {
        // ARRANGE
        let interfaces = vec![
            make_interface("enx001122334455", LinkStateKind::Up),
            make_interface("eth0", LinkStateKind::NoCarrier),
        ];

        // ACT
        let primary = InterfaceSelector::select_primary(&interfaces);

        // ASSERT
        assert_eq!(
            primary.expect("should have primary").name,
            "enx001122334455"
        );
    }

    #[test]
    fn carrier_overrides_naming() {
        // ARRANGE
        let interfaces = vec![
            make_interface("eno1", LinkStateKind::NoCarrier),
            make_interface("eth0", LinkStateKind::Up),
        ];

        // ACT
        let primary = InterfaceSelector::select_primary(&interfaces);

        // ASSERT
        assert_eq!(primary.expect("should have primary").name, "eth0");
    }

    #[test]
    fn select_backups_excludes_primary() {
        // ARRANGE
        let interfaces = vec![
            make_interface("eth0", LinkStateKind::Up),
            make_interface("eth1", LinkStateKind::Up),
            make_interface("eth2", LinkStateKind::Down),
        ];
        let primary_name = Name::new("eth0").unwrap();

        // ACT
        let backups = InterfaceSelector::select_backups(&interfaces, &primary_name);

        // ASSERT
        assert_eq!(backups.len(), 2);
        assert!(backups.iter().all(|i| i.name != "eth0"));
    }

    #[test]
    fn select_backups_sorted_by_priority() {
        // ARRANGE
        let interfaces = vec![
            make_interface("eth0", LinkStateKind::Up),
            make_interface("eth1", LinkStateKind::Down),
            make_interface("eno1", LinkStateKind::Up),
            make_interface("enp3s0", LinkStateKind::Up),
        ];
        let primary_name = Name::new("eth0").unwrap();

        // ACT
        let backups = InterfaceSelector::select_backups(&interfaces, &primary_name);
        let backup_names: Vec<_> = backups
            .iter()
            .map(|interface| interface.name.as_str())
            .collect();

        // ASSERT
        assert_eq!(backup_names.as_slice(), ["eno1", "enp3s0", "eth1"]);
    }

    #[test]
    fn empty_interfaces() {
        // ARRANGE
        let interfaces: Vec<Interface> = vec![];

        // ACT
        let primary = InterfaceSelector::select_primary(&interfaces);

        // ASSERT
        assert!(primary.is_none());
    }

    #[test]
    fn single_interface() {
        // ARRANGE
        let interfaces = vec![make_interface("eth0", LinkStateKind::Down)];

        // ACT
        let primary = InterfaceSelector::select_primary(&interfaces);

        // ASSERT
        assert_eq!(primary.expect("should have primary").name, "eth0");
    }

    #[test]
    fn has_carrier() {
        // ACT & ASSERT
        assert!(make_interface("eth0", LinkStateKind::Up).has_carrier());
        assert!(!make_interface("eth0", LinkStateKind::NoCarrier).has_carrier());
        assert!(!make_interface("eth0", LinkStateKind::Down).has_carrier());
    }

    #[test]
    fn tiebreaker_prefers_lower_index() {
        // ARRANGE
        let interfaces = vec![
            make_interface_with_index("eth0", 8, LinkStateKind::Down),
            make_interface_with_index("eth1", 9, LinkStateKind::Down),
            make_interface_with_index("eth2", 10, LinkStateKind::Down),
            make_interface_with_index("eth3", 11, LinkStateKind::Down),
        ];

        // ACT
        let primary = InterfaceSelector::select_primary(&interfaces);

        // ASSERT
        let primary_interface = primary.expect("should have primary");
        assert_eq!(primary_interface.name, "eth0");
        assert_eq!(primary_interface.index, 8);
    }
}
