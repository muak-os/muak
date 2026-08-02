use std::os::fd::OwnedFd;

use anyhow::{Context as _, Result};
use rustix::net::netlink::KOBJECT_UEVENT;
use rustix::net::netlink::SocketAddrNetlink;
use rustix::net::{AddressFamily, RecvFlags, SocketFlags, SocketType, bind, recv, socket_with};

const KOBJECT_UEVENT_GROUP: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UeventAction {
    Add,
    Remove,
    Other(String),
}

#[derive(Debug)]
pub struct Uevent {
    pub action: UeventAction,
    pub modalias: Option<String>,
    pub subsystem: Option<String>,
}

pub struct UeventListener {
    socket: OwnedFd,
}

impl UeventListener {
    pub fn new() -> Result<Self> {
        let socket = socket_with(
            AddressFamily::NETLINK,
            SocketType::DGRAM,
            SocketFlags::CLOEXEC,
            Some(KOBJECT_UEVENT),
        )
        .context("Failed to create netlink socket")?;

        let addr = SocketAddrNetlink::new(0, KOBJECT_UEVENT_GROUP);
        bind(&socket, &addr).context("Failed to bind netlink socket")?;

        Ok(Self { socket })
    }

    /// Reads the next uevent from the netlink socket.
    pub fn recv(&self) -> Result<Uevent> {
        let mut buf = [0_u8; 8192];

        let (bytes_initialized, _total_bytes) =
            recv(&self.socket, &mut buf[..], RecvFlags::empty())
                .context("Failed to receive uevent")?;

        Ok(parse_uevent(
            buf.get(..bytes_initialized).unwrap_or_default(),
        ))
    }
}

fn parse_uevent(data: &[u8]) -> Uevent {
    let mut event = Uevent {
        action: UeventAction::Other(String::new()),
        modalias: None,
        subsystem: None,
    };

    for part in data.split(|&byte| byte == 0) {
        let Ok(text) = core::str::from_utf8(part) else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        apply_part(text, &mut event);
    }

    event
}

fn apply_part(part: &str, event: &mut Uevent) {
    if let Some((act, _rest)) = part.split_once('@') {
        event.action = parse_action(act);
        return;
    }

    let Some((key, value)) = part.split_once('=') else {
        return;
    };

    match key {
        "ACTION" => event.action = parse_action(value),
        "MODALIAS" => event.modalias = Some(value.to_owned()),
        "SUBSYSTEM" => event.subsystem = Some(value.to_owned()),
        _ => {}
    }
}

fn parse_action(act: &str) -> UeventAction {
    match act {
        "add" => UeventAction::Add,
        "remove" => UeventAction::Remove,
        other => UeventAction::Other(other.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_uevent_add_with_modalias() {
        // ARRANGE
        let data = b"add@/devices/pci0000:00/0000:00:1f.6\0ACTION=add\0DEVPATH=/devices/pci0000:00/0000:00:1f.6\0SUBSYSTEM=pci\0MODALIAS=pci:v00008086d00001234\0";

        // ACT
        let event = parse_uevent(data);

        // ASSERT
        assert_eq!(event.action, UeventAction::Add);
        assert_eq!(event.modalias, Some("pci:v00008086d00001234".to_owned()));
        assert_eq!(event.subsystem, Some("pci".to_owned()));
    }

    #[test]
    fn parse_uevent_remove() {
        // ARRANGE
        let data = b"remove@/devices/usb/1-1\0ACTION=remove\0SUBSYSTEM=usb\0";

        // ACT
        let event = parse_uevent(data);

        // ASSERT
        assert_eq!(event.action, UeventAction::Remove);
        assert_eq!(event.modalias, None);
    }

    #[test]
    fn parse_uevent_no_modalias() {
        // ARRANGE
        let data = b"add@/devices/virtual/net/lo\0ACTION=add\0SUBSYSTEM=net\0";

        // ACT
        let event = parse_uevent(data);

        // ASSERT
        assert_eq!(event.action, UeventAction::Add);
        assert_eq!(event.modalias, None);
    }
}
