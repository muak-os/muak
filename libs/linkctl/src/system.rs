//! Address families selected by the enabled build features.

#[cfg(any(not(feature = "ipv4"), not(feature = "ipv6")))]
use crate::disabled::Family;
#[cfg(feature = "ipv4")]
use crate::interface::v4;
#[cfg(feature = "ipv6")]
use crate::interface::v6;
#[cfg(not(feature = "ipv4"))]
use crate::view::V4State;
#[cfg(not(feature = "ipv6"))]
use crate::view::V6State;

#[cfg(not(feature = "ipv4"))]
/// IPv4 family of this build.
pub type V4 = Family<addrconf::v4::Config, V4State>;

#[cfg(feature = "ipv4")]
/// IPv4 family of this build.
pub type V4 = v4::Family;

#[cfg(not(feature = "ipv6"))]
/// IPv6 family of this build.
pub type V6 = Family<addrconf::v6::Config, V6State>;

#[cfg(feature = "ipv6")]
/// IPv6 family of this build.
pub type V6 = v6::Family;
