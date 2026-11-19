//! RFC 2131 fixed-header field offsets.

pub(crate) const OP: usize = 0;
pub(crate) const HTYPE: usize = 1;
pub(crate) const HLEN: usize = 2;
pub(crate) const HOPS: usize = 3;
pub(crate) const XID: usize = 4;
pub(crate) const FLAGS: usize = 10;
pub(crate) const YIADDR: usize = 16;
pub(crate) const CHADDR: usize = 28;
pub(crate) const HEADER_LEN: usize = 236;
