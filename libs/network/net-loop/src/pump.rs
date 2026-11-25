//! Socket receive pump racing the next link event.

use alloc::vec::Vec;
use core::net::IpAddr;
use core::pin::pin;
use std::io;

use futures_util::StreamExt as _;
use futures_util::future::{Either, select};
use futures_util::stream::FuturesUnordered;
use net_runtime::{Events as _, Icmpv6 as _, Packet as _, Udp as _};

use crate::pending::Pending;
use crate::{Loop, Progress, SocketEntry, SocketTable, interface_effects};

const FRAME_BUDGET: usize = 2048;

type Payload = (Vec<u8>, Option<IpAddr>);

type Received = (net_runtime::SocketId, io::Result<Payload>);

enum Next {
    Frame(Received),
    Link(netmodel::LinkEvent),
}

/// Waits for the next socket frame or, when no socket has one, the next link event.
///
/// # Errors
///
/// Returns the error the link event port reported.
pub(crate) async fn wait_event<R: net_runtime::Runtime>(
    driver: &mut Loop<R>,
) -> io::Result<Progress> {
    match next(&driver.runtime, &driver.sockets).await? {
        Next::Frame(received) => queue(driver, received).await,
        Next::Link(event) => driver.pending.push_back(Pending::LinkEvent(event)),
    }

    Ok(Progress::Worked)
}

async fn next<R: net_runtime::Runtime>(runtime: &R, sockets: &SocketTable) -> io::Result<Next> {
    if sockets.is_empty() {
        return runtime.events().next().await.map(Next::Link);
    }

    let frames = pin!(first(runtime, sockets));
    let events = pin!(runtime.events().next());

    match select(frames, events).await {
        Either::Left((Some(received), _)) => Ok(Next::Frame(received)),
        Either::Left((None, events)) => events.await.map(Next::Link),
        Either::Right((event, _)) => event.map(Next::Link),
    }
}

async fn queue<R: net_runtime::Runtime>(driver: &mut Loop<R>, received: Received) {
    let (id, result) = received;
    let Some(owner) = driver.sockets.get(id) else {
        return;
    };
    let name = owner.interface.clone();
    let family = owner.family;
    let slot = owner.slot;
    let kind = owner.kind;
    let queued = match result {
        Ok((bytes, source)) => Pending::Frame {
            name,
            family,
            slot,
            bytes,
            source,
        },
        Err(error) => {
            drop(interface_effects::release(&driver.runtime, &mut driver.sockets, id, kind).await);
            Pending::SocketError {
                name,
                family,
                slot,
                error,
            }
        }
    };
    driver.pending.push_back(queued);
}

async fn first<R: net_runtime::Runtime>(runtime: &R, sockets: &SocketTable) -> Option<Received> {
    let mut pending = FuturesUnordered::new();
    for entry in sockets.iter() {
        pending.push(receive(runtime, entry));
    }

    while let Some(received) = pending.next().await {
        if matches!(&received.1, Err(error) if error.kind() == io::ErrorKind::WouldBlock) {
            continue;
        }

        return Some(received);
    }

    None
}

async fn receive<R: net_runtime::Runtime>(runtime: &R, entry: &SocketEntry) -> Received {
    let mut buffer = [0_u8; FRAME_BUDGET];
    let result = match entry.owner.kind {
        ip_method::SocketKind::RawL2 => runtime
            .packet()
            .recv(entry.id, &mut buffer)
            .await
            .map(|len| payload(&buffer, len, None)),
        ip_method::SocketKind::Udp => runtime
            .udp()
            .recv(entry.id, &mut buffer)
            .await
            .map(|len| payload(&buffer, len, None)),
        ip_method::SocketKind::RawIcmpv6 => runtime
            .icmpv6()
            .recv(entry.id, &mut buffer)
            .await
            .map(|(len, source)| payload(&buffer, len, source.map(IpAddr::V6))),
    };

    (entry.id, result)
}

fn payload(buffer: &[u8], len: usize, source: Option<IpAddr>) -> Payload {
    let bytes = buffer.get(..len).unwrap_or_default().to_vec();

    (bytes, source)
}
