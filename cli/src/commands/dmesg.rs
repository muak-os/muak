use anyhow::Result;
use tonic::transport::Channel;

use crate::client::log_service::log_service_client::LogServiceClient;
use crate::commands::logs;
use crate::console::Console;

/// Streams kernel logs.
pub async fn handle(
    client: &mut LogServiceClient<Channel>,
    follow: bool,
    console: &Console,
) -> Result<()> {
    logs::handle(
        client,
        Some("kernel".to_owned()),
        None,
        follow,
        None,
        console,
    )
    .await
}
