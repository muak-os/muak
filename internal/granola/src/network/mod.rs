pub mod bridge;
pub mod config;
pub mod dhcp;
pub mod dns;
pub mod interface;
pub mod tap; // still used by VM creation

// Refactored components
pub mod model;
pub mod ops;
pub mod actor;

pub use tap::{format_mac_address, generate_mac_address};

pub use actor::{start_network_actor, NetworkActorHandle};
