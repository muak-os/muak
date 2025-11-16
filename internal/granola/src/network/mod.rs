pub mod config;
pub mod dhcp;
pub mod dns;
pub mod interface;

pub mod model;
pub mod monitor;

pub mod netlink;
pub mod services;

pub mod actor;

pub use actor::{NetworkActorHandle, start_network_actor};
pub use services::tap::{format_mac_address, generate_mac_address};
