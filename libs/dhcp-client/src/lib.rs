//! `DHCPv4` lease acquisition client.

#![warn(missing_docs)]

extern crate alloc;

pub mod datagram;
pub mod error;
pub mod exchange;
pub mod lease;
pub mod method;
pub mod system;

/// Largest DHCP message the builders produce.
pub const MAX_MESSAGE_LEN: usize = 1500;
/// Largest IP frame needed to carry [`MAX_MESSAGE_LEN`].
pub const MAX_FRAME_LEN: usize = 1536;

/// Caller supplied buffers for one exchange: the DHCP message and the IP frame around it.
pub struct Scratch<'a> {
    /// DHCP message buffer.
    pub msg: &'a mut [u8],
    /// IP frame buffer, large enough for an IPv4 and UDP header plus [`MAX_MESSAGE_LEN`].
    pub frame: &'a mut [u8],
}

impl<'a> Scratch<'a> {
    /// Pairs a message buffer with a frame buffer.
    #[must_use = "the message and frame buffers must be used together"]
    pub const fn new(msg: &'a mut [u8], frame: &'a mut [u8]) -> Self {
        Self { msg, frame }
    }
}
