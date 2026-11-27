//! Link control port over rtnetlink.

use netmodel::Name;

use crate::control;
use crate::error::TokioError;

impl net_runtime::Link for super::Tokio {
    async fn detach(&self, port: &Name) -> net_runtime::error::Result<()> {
        let index = net_sys_linux::index::resolve(port).map_err(TokioError::from)?;

        Ok(control::execute(&self.link, netlink_linux::detach(index)).await?)
    }

    async fn ensure_bridge(&self, name: &Name, stp: bool) -> net_runtime::error::Result<()> {
        match net_sys_linux::index::resolve(name) {
            Ok(_) => return Ok(()),
            Err(net_sys_linux::error::NetSysError::NotFound(_)) => {}
            Err(error) => return Err(TokioError::from(error).into()),
        }

        Ok(control::execute(&self.link, netlink_linux::create_bridge(name, stp)).await?)
    }

    async fn link(&self, name: &Name) -> net_runtime::error::Result<Option<netmodel::Link>> {
        let index = match net_sys_linux::index::resolve(name) {
            Ok(index) => index,
            Err(net_sys_linux::error::NetSysError::NotFound(_)) => return Ok(None),
            Err(error) => return Err(TokioError::from(error).into()),
        };
        let replies = match control::collect(&self.link, netlink_linux::get_link(index)).await {
            Ok(replies) => replies,
            Err(TokioError::Io(error)) if error.raw_os_error() == Some(libc::ENODEV) => {
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        };

        Ok(replies.iter().find_map(netlink_linux::parse_link))
    }

    async fn links(&self) -> net_runtime::error::Result<Vec<netmodel::Link>> {
        Ok(control::links(&self.link).await?)
    }

    async fn set_down(&self, name: &Name) -> net_runtime::error::Result<()> {
        let index = net_sys_linux::index::resolve(name).map_err(TokioError::from)?;

        Ok(control::execute(&self.link, netlink_linux::set_link_down(index)).await?)
    }

    async fn set_master(&self, port: &Name, master: &Name) -> net_runtime::error::Result<()> {
        let port = net_sys_linux::index::resolve(port).map_err(TokioError::from)?;
        let master = net_sys_linux::index::resolve(master).map_err(TokioError::from)?;

        Ok(control::execute(&self.link, netlink_linux::set_master(port, master)).await?)
    }

    async fn set_up(&self, name: &Name) -> net_runtime::error::Result<()> {
        let index = net_sys_linux::index::resolve(name).map_err(TokioError::from)?;

        Ok(control::execute(&self.link, netlink_linux::set_link_up(index)).await?)
    }
}
