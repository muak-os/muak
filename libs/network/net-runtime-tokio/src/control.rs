//! Shared rtnetlink connection creation, requests, and replies.

use std::io;

use futures_channel::mpsc::UnboundedReceiver;
use futures_util::StreamExt as _;
use netlink_linux::Message;
use netlink_packet_core::{ErrorMessage, NetlinkMessage, NetlinkPayload};
use netlink_packet_route::RouteNetlinkMessage;
use netlink_proto::sys::{SocketAddr, protocols};
use netlink_proto::{Connection, ConnectionHandle, Error};
use netlink_sys::{AsyncSocket as _, TokioSocket};

const RTNLGRP_LINK: u32 = 1;

pub(crate) type Handle = ConnectionHandle<RouteNetlinkMessage>;
pub(crate) type Messages = UnboundedReceiver<(NetlinkMessage<RouteNetlinkMessage>, SocketAddr)>;

/// Multicast mask for the route netlink link notification group.
pub(crate) const fn link_membership() -> u32 {
    1 << (RTNLGRP_LINK - 1)
}

pub(crate) fn connect() -> io::Result<(Handle, Messages)> {
    let (mut connection, handle, messages) = netlink_proto::new_connection_with_socket::<
        RouteNetlinkMessage,
        TokioSocket,
    >(protocols::NETLINK_ROUTE)?;
    subscribe(&mut connection)?;
    tokio::spawn(connection);

    Ok((handle, messages))
}

pub(crate) async fn execute(handle: &Handle, message: Message) -> io::Result<()> {
    let mut responses = handle.request(message, kernel()).map_err(request_error)?;
    while let Some(message) = responses.next().await {
        check(message.payload)?;
    }

    Ok(())
}

pub(crate) async fn collect(
    handle: &Handle,
    message: Message,
) -> io::Result<Vec<RouteNetlinkMessage>> {
    let mut responses = handle.request(message, kernel()).map_err(request_error)?;
    let mut replies = Vec::new();
    while let Some(message) = responses.next().await {
        if let NetlinkPayload::Error(error) = message.payload {
            reject(&error)?;
            continue;
        }
        if let NetlinkPayload::InnerMessage(inner) = message.payload {
            replies.push(inner);
        }
    }

    Ok(replies)
}

pub(crate) async fn links(handle: &Handle) -> io::Result<Vec<netmodel::Link>> {
    let replies = collect(handle, netlink_linux::get_links()).await?;

    Ok(replies
        .iter()
        .filter_map(netlink_linux::parse_link)
        .collect())
}

pub(crate) fn resolve(name: &netmodel::Name) -> io::Result<u32> {
    let text = name.as_str();
    let mut buffer = [0_u8; 16];
    for (slot, byte) in buffer.iter_mut().zip(text.bytes()) {
        *slot = byte;
    }
    // SAFETY: the buffer is 16 bytes, the name is at most 15 bytes, so the
    // copied name is NUL terminated and the pointer is valid for the call.
    let index = unsafe { libc::if_nametoindex(buffer.as_ptr().cast()) };
    if index == 0 {
        return Err(missing(name, io::Error::last_os_error()));
    }

    Ok(index)
}

pub(crate) fn tolerate(result: io::Result<()>, kind: io::ErrorKind) -> io::Result<()> {
    match result {
        Err(error) if error.kind() == kind => Ok(()),
        other => other,
    }
}

fn missing(name: &netmodel::Name, error: io::Error) -> io::Error {
    match error.raw_os_error() {
        Some(libc::ENODEV | libc::ENXIO) => io::Error::new(
            io::ErrorKind::NotFound,
            format!("link {name} does not exist"),
        ),
        _ => error,
    }
}

fn subscribe(connection: &mut Connection<RouteNetlinkMessage, TokioSocket>) -> io::Result<()> {
    connection
        .socket_mut()
        .socket_mut()
        .bind(&SocketAddr::new(0, link_membership()))
}

fn kernel() -> SocketAddr {
    SocketAddr::new(0, 0)
}

fn check(payload: NetlinkPayload<RouteNetlinkMessage>) -> io::Result<()> {
    let NetlinkPayload::Error(error) = payload else {
        return Ok(());
    };

    reject(&error)
}

fn reject(error: &ErrorMessage) -> io::Result<()> {
    if error.code.is_some() {
        return Err(error.to_io());
    }

    Ok(())
}

fn request_error(error: Error<RouteNetlinkMessage>) -> io::Error {
    match error {
        Error::ConnectionClosed => {
            io::Error::new(io::ErrorKind::BrokenPipe, "netlink connection closed")
        }
        Error::NetlinkError(_message) => io::Error::other("netlink rejected the request"),
        Error::SocketIo(error) => error,
    }
}

#[cfg(test)]
mod tests {
    use core::num::NonZeroI32;

    use netlink_packet_core::{ErrorBuffer, Parseable as _};

    use super::*;

    fn nack(code: i32) -> ErrorMessage {
        let bytes = code.to_le_bytes();
        let buffer = ErrorBuffer::new_checked(&bytes).expect("error buffer");

        ErrorMessage::parse(&buffer).expect("parsed error message")
    }

    #[tokio::test]
    async fn subscription_joins_the_link_group() {
        // ARRANGE
        let (mut connection, _handle, _messages) = netlink_proto::new_connection_with_socket::<
            RouteNetlinkMessage,
            TokioSocket,
        >(protocols::NETLINK_ROUTE)
        .expect("netlink connection");

        // ACT
        let result = subscribe(&mut connection);
        let mut address = SocketAddr::new(0, 0);
        connection
            .socket_mut()
            .socket_mut()
            .get_address(&mut address)
            .expect("bound socket address");

        // ASSERT
        assert_eq!(link_membership(), 1);
        result.expect("link subscription binds");
        assert_eq!(address.multicast_groups(), link_membership());
    }

    #[test]
    fn resolve_finds_loopback() {
        // ARRANGE
        let name = netmodel::Name::new("lo").expect("loopback name");

        // ACT
        let result = resolve(&name);

        // ASSERT
        assert!(result.expect("loopback resolves") > 0);
    }

    #[test]
    fn resolve_rejects_missing_link() {
        // ARRANGE
        let name = netmodel::Name::new("nomissing0").expect("valid name");

        // ACT
        let result = resolve(&name);

        // ASSERT
        assert!(matches!(
            result,
            Err(error) if error.kind() == io::ErrorKind::NotFound
        ));
    }

    #[test]
    fn acknowledgment_is_success() {
        // ARRANGE
        let payload = NetlinkPayload::Error(ErrorMessage::default());

        // ACT
        let result = check(payload);

        // ASSERT
        result.expect("acknowledgment is accepted");
    }

    #[test]
    fn kernel_rejection_becomes_an_io_error() {
        // ARRANGE
        let code = NonZeroI32::new(17_i32).expect("nonzero errno");
        let payload = NetlinkPayload::Error(nack(-code.get()));

        // ACT
        let result = check(payload);

        // ASSERT
        assert!(matches!(
            result,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists
        ));
    }
}
