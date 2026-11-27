//! Address management port over rtnetlink.

use std::io;

use netlink_packet_route::RouteNetlinkMessage;
use netmodel::{Address, Name};

use crate::control;

impl net_runtime::Address for super::Tokio {
    async fn add(&self, link: &Name, address: Address) -> io::Result<()> {
        let index = net_sys_linux::index::resolve(link)?;

        control::tolerate(
            control::execute(
                &self.address,
                netlink_linux::add_address(index, address.address, address.prefix_len),
            )
            .await,
            io::ErrorKind::AlreadyExists,
        )
    }

    async fn list(&self, link: &Name) -> io::Result<Vec<Address>> {
        let index = net_sys_linux::index::resolve(link)?;
        let replies = control::collect(&self.address, netlink_linux::get_addresses()).await?;

        Ok(replies
            .iter()
            .filter_map(|message| owned_by(message, index))
            .collect())
    }

    async fn remove(&self, link: &Name, address: Address) -> io::Result<()> {
        let index = net_sys_linux::index::resolve(link)?;

        control::tolerate(
            control::execute(
                &self.address,
                netlink_linux::remove_address(index, address.address, address.prefix_len),
            )
            .await,
            io::ErrorKind::NotFound,
        )
    }
}

fn owned_by(message: &RouteNetlinkMessage, index: u32) -> Option<Address> {
    let (owner, address) = netlink_linux::parse_address_index(message)?;

    (owner == index).then_some(address)
}
