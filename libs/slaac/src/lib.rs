//! IPv6 stateless address autoconfiguration: `ICMPv6` router discovery, address generation, and
//! the RFC 4862 address method.

#![warn(missing_docs)]

extern crate alloc;

pub mod address;
pub mod icmpv6;
pub mod method;
