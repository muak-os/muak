//! Default route management port over rtnetlink.

use core::net::IpAddr;
use std::io;

use netmodel::Name;

use crate::control;

impl net_runtime::Route for super::Tokio {
    async fn ensure_default(&self, link: &Name, gateway: IpAddr) -> io::Result<()> {
        let index = net_sys_linux::index::resolve(link)?;

        control::tolerate(
            control::execute(&self.route, netlink_linux::add_route(index, gateway)).await,
            io::ErrorKind::AlreadyExists,
        )
    }

    async fn remove_default(&self, link: &Name, gateway: IpAddr) -> io::Result<()> {
        let index = net_sys_linux::index::resolve(link)?;

        control::tolerate(
            control::execute(&self.route, netlink_linux::remove_route(index, gateway)).await,
            io::ErrorKind::NotFound,
        )
    }
}
