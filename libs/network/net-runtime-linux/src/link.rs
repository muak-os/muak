//! Link control port over rtnetlink.

use std::io;

use netmodel::Name;

use crate::control;

impl net_runtime::Link for super::Tokio {
    async fn detach(&self, port: &Name) -> io::Result<()> {
        let index = control::resolve(port)?;

        control::execute(&self.link, netlink_linux::detach(index)).await
    }

    async fn ensure_bridge(&self, name: &Name, stp: bool) -> io::Result<()> {
        match control::resolve(name) {
            Ok(_) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }

        control::execute(&self.link, netlink_linux::create_bridge(name, stp)).await
    }

    async fn link(&self, name: &Name) -> io::Result<Option<netmodel::Link>> {
        let index = match control::resolve(name) {
            Ok(index) => index,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let replies = match control::collect(&self.link, netlink_linux::get_link(index)).await {
            Ok(replies) => replies,
            Err(error) if error.raw_os_error() == Some(libc::ENODEV) => return Ok(None),
            Err(error) => return Err(error),
        };

        Ok(replies.iter().find_map(netlink_linux::parse_link))
    }

    async fn links(&self) -> io::Result<Vec<netmodel::Link>> {
        control::links(&self.link).await
    }

    async fn set_down(&self, name: &Name) -> io::Result<()> {
        let index = control::resolve(name)?;

        control::execute(&self.link, netlink_linux::set_link_down(index)).await
    }

    async fn set_master(&self, port: &Name, master: &Name) -> io::Result<()> {
        let port = control::resolve(port)?;
        let master = control::resolve(master)?;

        control::execute(&self.link, netlink_linux::set_master(port, master)).await
    }

    async fn set_up(&self, name: &Name) -> io::Result<()> {
        let index = control::resolve(name)?;

        control::execute(&self.link, netlink_linux::set_link_up(index)).await
    }
}
