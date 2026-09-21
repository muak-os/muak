//! Uevent parsing over a netlink socket, borrowing fields from an internal buffer.

use std::os::fd::OwnedFd;

use anyhow::{Context as _, Result};
use rustix::net::netlink::KOBJECT_UEVENT;
use rustix::net::netlink::SocketAddrNetlink;
use rustix::net::{AddressFamily, RecvFlags, SocketFlags, SocketType, bind, recv, socket_with};

const KOBJECT_UEVENT_GROUP: u32 = 1;

const UEVENT_MAX_SIZE: usize = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UeventAction<'a> {
    Add,
    Remove,
    Other(&'a str),
}

#[derive(Debug)]
pub struct Uevent<'a> {
    pub action: UeventAction<'a>,
    pub modalias: Option<&'a str>,
    pub subsystem: Option<&'a str>,
}

pub struct UeventListener {
    socket: OwnedFd,
    buf: [u8; UEVENT_MAX_SIZE],
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

        Ok(Self {
            socket,
            buf: [0_u8; UEVENT_MAX_SIZE],
        })
    }

    /// Reads the next uevent from the netlink socket.
    pub fn recv(&mut self) -> Result<Uevent<'_>> {
        let (bytes_initialized, _total_bytes) =
            recv(&self.socket, &mut self.buf[..], RecvFlags::empty())
                .context("Failed to receive uevent")?;

        let data = self.buf.get(..bytes_initialized).unwrap_or_default();

        Ok(parse_uevent(data))
    }
}

fn parse_uevent(data: &[u8]) -> Uevent<'_> {
    let mut parts = data.split(|&byte| byte == 0).filter_map(|part| {
        core::str::from_utf8(part)
            .ok()
            .filter(|text| !text.is_empty())
    });

    let Some(header) = parts.next() else {
        return Uevent {
            action: UeventAction::Other(""),
            modalias: None,
            subsystem: None,
        };
    };

    let action = header
        .split_once('@')
        .map_or(UeventAction::Other(header), |(act, _)| parse_action(act));

    if !matches!(action, UeventAction::Add) {
        return Uevent {
            action,
            modalias: None,
            subsystem: None,
        };
    }

    let mut event = Uevent {
        action,
        modalias: None,
        subsystem: None,
    };
    (event.modalias, event.subsystem) = scan_keys(parts);
    event
}

fn scan_keys<'a>(parts: impl Iterator<Item = &'a str>) -> (Option<&'a str>, Option<&'a str>) {
    let mut modalias = None;
    let mut subsystem = None;

    for text in parts {
        let Some((key, value)) = text.split_once('=') else {
            continue;
        };
        match key {
            "MODALIAS" => modalias = Some(value),
            "SUBSYSTEM" => subsystem = Some(value),
            _ => {}
        }
    }

    (modalias, subsystem)
}

fn parse_action(act: &str) -> UeventAction<'_> {
    match act {
        "add" => UeventAction::Add,
        "remove" => UeventAction::Remove,
        other => UeventAction::Other(other),
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
        assert_eq!(event.modalias, Some("pci:v00008086d00001234"));
        assert_eq!(event.subsystem, Some("pci"));
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

    #[test]
    fn parse_uevent_unknown_action() {
        // ARRANGE
        let data = b"change@/devices/usb/1-1\0ACTION=change\0MODALIAS=usb:v046D\0";

        // ACT
        let event = parse_uevent(data);

        // ASSERT
        assert_eq!(event.action, UeventAction::Other("change"));
        assert_eq!(event.modalias, None);
    }

    #[test]
    fn parse_uevent_empty_message() {
        // ACT
        let event = parse_uevent(b"");

        // ASSERT
        assert_eq!(event.action, UeventAction::Other(""));
        assert_eq!(event.modalias, None);
        assert_eq!(event.subsystem, None);
    }
}
