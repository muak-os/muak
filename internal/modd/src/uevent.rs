use anyhow::{Context, Result};
use std::io::Error;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

const NETLINK_KOBJECT_UEVENT: i32 = 15;
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

#[repr(C)]
struct SockaddrNl {
    nl_family: u16,
    nl_pad: u16,
    nl_pid: u32,
    nl_groups: u32,
}

impl UeventListener {
    pub fn new() -> Result<Self> {
        let fd = unsafe {
            nix::libc::socket(
                nix::libc::AF_NETLINK,
                nix::libc::SOCK_DGRAM,
                NETLINK_KOBJECT_UEVENT,
            )
        };

        if fd < 0 {
            return Err(Error::last_os_error()).context("Failed to create netlink socket");
        }

        let socket = unsafe { OwnedFd::from_raw_fd(fd) };

        let addr = SockaddrNl {
            nl_family: nix::libc::AF_NETLINK as u16,
            nl_pad: 0,
            nl_pid: 0,
            nl_groups: KOBJECT_UEVENT_GROUP,
        };

        let ret = unsafe {
            nix::libc::bind(
                socket.as_raw_fd(),
                &addr as *const _ as *const nix::libc::sockaddr,
                std::mem::size_of::<SockaddrNl>() as u32,
            )
        };

        if ret < 0 {
            return Err(Error::last_os_error()).context("Failed to bind netlink socket");
        }

        Ok(Self { socket })
    }

    pub fn recv(&self) -> Result<Uevent> {
        let mut buf = [0u8; 8192];

        let n = unsafe {
            nix::libc::recv(
                self.socket.as_raw_fd(),
                buf.as_mut_ptr() as *mut _,
                buf.len(),
                0,
            )
        };

        if n < 0 {
            return Err(Error::last_os_error()).context("Failed to receive uevent");
        }

        let data = &buf[..n as usize];
        Ok(parse_uevent(data))
    }
}

fn parse_uevent(data: &[u8]) -> Uevent {
    let mut action = UeventAction::Other(String::new());
    let mut modalias = None;
    let mut subsystem = None;

    for part in data.split(|&b| b == 0) {
        if part.is_empty() {
            continue;
        }

        let Ok(s) = std::str::from_utf8(part) else {
            continue;
        };

        if let Some((act, _rest)) = s.split_once('@') {
            action = match act {
                "add" => UeventAction::Add,
                "remove" => UeventAction::Remove,
                other => UeventAction::Other(other.to_string()),
            };
            continue;
        }

        if let Some((key, value)) = s.split_once('=') {
            match key {
                "ACTION" => {
                    action = match value {
                        "add" => UeventAction::Add,
                        "remove" => UeventAction::Remove,
                        other => UeventAction::Other(other.to_string()),
                    };
                }
                "MODALIAS" => {
                    modalias = Some(value.to_string());
                }
                "SUBSYSTEM" => {
                    subsystem = Some(value.to_string());
                }
                _ => {}
            }
        }
    }

    Uevent {
        action,
        modalias,
        subsystem,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_uevent_add_with_modalias() {
        let data = b"add@/devices/pci0000:00/0000:00:1f.6\0ACTION=add\0DEVPATH=/devices/pci0000:00/0000:00:1f.6\0SUBSYSTEM=pci\0MODALIAS=pci:v00008086d00001234\0";
        let event = parse_uevent(data);

        assert_eq!(event.action, UeventAction::Add);
        assert_eq!(event.modalias, Some("pci:v00008086d00001234".to_string()));
        assert_eq!(event.subsystem, Some("pci".to_string()));
    }

    #[test]
    fn test_parse_uevent_remove() {
        let data = b"remove@/devices/usb/1-1\0ACTION=remove\0SUBSYSTEM=usb\0";
        let event = parse_uevent(data);

        assert_eq!(event.action, UeventAction::Remove);
        assert_eq!(event.modalias, None);
    }

    #[test]
    fn test_parse_uevent_no_modalias() {
        let data = b"add@/devices/virtual/net/lo\0ACTION=add\0SUBSYSTEM=net\0";
        let event = parse_uevent(data);

        assert_eq!(event.action, UeventAction::Add);
        assert_eq!(event.modalias, None);
    }
}
