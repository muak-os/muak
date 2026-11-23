//! Fixtures shared by the netlink-linux integration tests.

use netlink_linux::Message;

pub(crate) fn emitted(message: &Message) -> Vec<u8> {
    let mut buffer = vec![0_u8; message.buffer_len()];
    message.serialize(&mut buffer);
    buffer
}
