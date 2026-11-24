//! Unsolicited rtnetlink notifications mapped to domain link events.

use std::collections::HashMap;
use std::io;

use futures_util::StreamExt as _;
use netlink_packet_core::NetlinkPayload;
use netlink_packet_route::RouteNetlinkMessage;
use netmodel::{LinkEvent, LinkFailure, LinkState};
use tokio::sync::Mutex;

use crate::control::{self, Messages};

pub(crate) struct Events {
    state: Mutex<State>,
}

struct State {
    messages: Messages,
    handle: control::Handle,
    previous: HashMap<u32, LinkState>,
    seeded: bool,
    closed: bool,
}

impl Events {
    pub(crate) fn new(messages: Messages, handle: control::Handle) -> Self {
        Self {
            state: Mutex::new(State {
                messages,
                handle,
                previous: HashMap::new(),
                seeded: false,
                closed: false,
            }),
        }
    }

    pub(crate) async fn next(&self) -> io::Result<LinkEvent> {
        drain(&self.state).await
    }
}

impl net_runtime::Events for super::Tokio {
    fn next(&self) -> impl Future<Output = io::Result<LinkEvent>> + Send {
        self.events.next()
    }
}

async fn drain(state: &Mutex<State>) -> io::Result<LinkEvent> {
    let mut state = state.lock().await;
    seed(&mut state).await?;

    loop {
        if let Some(event) = step(&mut state).await? {
            return Ok(event);
        }
    }
}

async fn seed(state: &mut State) -> io::Result<()> {
    if state.seeded {
        return Ok(());
    }

    let links = control::links(&state.handle).await?;
    state.previous = links
        .into_iter()
        .map(|link| (link.index, link.state))
        .collect();
    state.seeded = true;

    Ok(())
}

async fn step(state: &mut State) -> io::Result<Option<LinkEvent>> {
    match state.messages.next().await {
        Some((message, _source)) => Ok(classify(message.payload, &mut state.previous)),
        None => closed(state),
    }
}

fn closed(state: &mut State) -> io::Result<Option<LinkEvent>> {
    if state.closed {
        return Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "netlink event stream closed",
        ));
    }

    state.closed = true;

    Ok(Some(LinkEvent::Error(LinkFailure::Closed)))
}

fn classify(
    payload: NetlinkPayload<RouteNetlinkMessage>,
    previous: &mut HashMap<u32, LinkState>,
) -> Option<LinkEvent> {
    if let NetlinkPayload::Error(error) = payload {
        return error
            .code
            .map(|code| LinkEvent::Error(LinkFailure::Kernel(code)));
    }
    if let NetlinkPayload::Overrun(_) = payload {
        return Some(LinkEvent::Error(LinkFailure::Overrun));
    }
    if let NetlinkPayload::InnerMessage(message) = payload {
        return classify_inner(&message, previous);
    }

    None
}

fn classify_inner(
    message: &RouteNetlinkMessage,
    previous: &mut HashMap<u32, LinkState>,
) -> Option<LinkEvent> {
    if let RouteNetlinkMessage::DelLink(ref reply) = *message {
        previous.remove(&reply.header.index);
    }

    let state = netlink_linux::parse_link(message).map(|link| (link.index, link.state));
    let known = state.and_then(|(index, _)| previous.get(&index).copied());
    let event = netlink_linux::parse_event(message, known);

    if let Some((index, link_state)) = state {
        previous.insert(index, link_state);
    }

    event
}

#[cfg(test)]
mod tests {
    use core::num::NonZeroI32;

    use netlink_packet_core::{ErrorBuffer, ErrorMessage, Parseable as _};

    use super::*;

    fn nack(code: i32) -> ErrorMessage {
        let bytes = code.to_le_bytes();
        let buffer = ErrorBuffer::new_checked(&bytes).expect("error buffer");

        ErrorMessage::parse(&buffer).expect("parsed error message")
    }

    #[test]
    fn overrun_is_reported_as_a_link_failure() {
        // ARRANGE
        let mut previous = HashMap::new();

        // ACT
        let event = classify(NetlinkPayload::Overrun(Vec::new()), &mut previous);

        // ASSERT
        assert_eq!(event, Some(LinkEvent::Error(LinkFailure::Overrun)));
    }

    #[test]
    fn kernel_rejection_is_reported_as_a_link_failure() {
        // ARRANGE
        let code = NonZeroI32::new(-17_i32).expect("nonzero errno");
        let mut previous = HashMap::new();

        // ACT
        let event = classify(NetlinkPayload::Error(nack(code.get())), &mut previous);

        // ASSERT
        assert_eq!(event, Some(LinkEvent::Error(LinkFailure::Kernel(code))));
    }

    #[test]
    fn acknowledgment_is_ignored() {
        // ARRANGE
        let mut previous = HashMap::new();

        // ACT
        let event = classify(
            NetlinkPayload::Error(ErrorMessage::default()),
            &mut previous,
        );

        // ASSERT
        assert_eq!(event, None);
    }
}
