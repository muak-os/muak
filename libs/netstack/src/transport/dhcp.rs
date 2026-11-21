//! Production `DHCPv4` exchange backed by kernel sockets.

use core::net::Ipv4Addr;

use dhcp_client::connector::{Connector as _, System as Kernel};
use dhcp_client::lease::Lease;
use dhcp_client::{MAX_FRAME_LEN, MAX_MESSAGE_LEN, Scratch};

use crate::provision::v4::Exchange;

/// Production `DHCPv4` exchange over the kernel.
pub struct System {
    connector: Kernel,
}

impl System {
    /// Opens the exchange for `interface`.
    ///
    /// # Errors
    ///
    /// Returns an error when the raw packet socket cannot be opened on `interface`.
    pub async fn open(interface: &str) -> dhcp_client::Result<Self> {
        let connector = Kernel;
        // Probe once so missing interfaces and permissions fail setup instead of acquisition;
        // each exchange opens the socket it needs.
        connector.create_raw(interface).await?;

        Ok(Self { connector })
    }
}

impl Exchange for System {
    async fn acquire(&self, interface: &str, mac: [u8; 6]) -> dhcp_client::Result<Lease> {
        let socket = self.connector.create_raw(interface).await?;
        let mut msg = [0_u8; MAX_MESSAGE_LEN];
        let mut frame = [0_u8; MAX_FRAME_LEN];

        dhcp_client::exchange::acquire(&socket, mac, Scratch::new(&mut msg, &mut frame)).await
    }

    async fn rebind(
        &self,
        interface: &str,
        mac: [u8; 6],
        server: Ipv4Addr,
        assigned: Ipv4Addr,
    ) -> dhcp_client::Result<Lease> {
        let socket = self.connector.create_raw(interface).await?;
        let mut msg = [0_u8; MAX_MESSAGE_LEN];
        let mut frame = [0_u8; MAX_FRAME_LEN];

        dhcp_client::exchange::rebind(
            &socket,
            mac,
            server,
            assigned,
            Scratch::new(&mut msg, &mut frame),
        )
        .await
    }

    async fn renew(
        &self,
        interface: &str,
        mac: [u8; 6],
        server: Ipv4Addr,
        assigned: Ipv4Addr,
    ) -> dhcp_client::Result<Lease> {
        let socket = self.connector.create_unicast(interface, assigned).await?;
        let mut msg = [0_u8; MAX_MESSAGE_LEN];
        let mut frame = [0_u8; MAX_FRAME_LEN];

        dhcp_client::exchange::renew(
            &socket,
            mac,
            server,
            assigned,
            Scratch::new(&mut msg, &mut frame),
        )
        .await
    }
}
