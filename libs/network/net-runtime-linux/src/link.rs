//! Link control port over rtnetlink.

use std::io;

use netmodel::Name;

use crate::control;

impl net_runtime::Link for super::Tokio {
    async fn detach(&self, port: &Name) -> io::Result<()> {
        let index = control::resolve(&self.link, port).await?;
        control::execute(&self.link, netlink_linux::detach(index)).await
    }

    async fn ensure_bridge(&self, name: &Name, stp: bool) -> io::Result<()> {
        let exists = control::links(&self.link)
            .await?
            .iter()
            .any(|link| &link.name == name);
        if exists {
            return Ok(());
        }

        control::execute(&self.link, netlink_linux::create_bridge(name, stp)).await
    }

    async fn link(&self, name: &Name) -> io::Result<Option<netmodel::Link>> {
        let link = control::links(&self.link)
            .await?
            .into_iter()
            .find(|link| &link.name == name);

        Ok(link)
    }

    async fn links(&self) -> io::Result<Vec<netmodel::Link>> {
        control::links(&self.link).await
    }

    async fn set_down(&self, name: &Name) -> io::Result<()> {
        let index = control::resolve(&self.link, name).await?;
        control::execute(&self.link, netlink_linux::set_link_down(index)).await
    }

    async fn set_master(&self, port: &Name, master: &Name) -> io::Result<()> {
        let port = control::resolve(&self.link, port).await?;
        let master = control::resolve(&self.link, master).await?;
        control::execute(&self.link, netlink_linux::set_master(port, master)).await
    }

    async fn set_up(&self, name: &Name) -> io::Result<()> {
        let index = control::resolve(&self.link, name).await?;
        control::execute(&self.link, netlink_linux::set_link_up(index)).await
    }
}
