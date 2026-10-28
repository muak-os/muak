//! RFC 2131 fixed-header field offsets.

pub const OP: usize = 0;
pub const HTYPE: usize = 1;
pub const HLEN: usize = 2;
pub const HOPS: usize = 3;
pub const XID: usize = 4;
pub const FLAGS: usize = 10;
pub const YIADDR: usize = 16;
pub const CHADDR: usize = 28;
pub const HEADER_LEN: usize = 236;
