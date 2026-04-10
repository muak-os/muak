use anyhow::Result;
use tonic::transport::Channel;

use crate::client::LogServiceClient;
use crate::commands::logs;

/// Streams kernel logs.
pub async fn handle(client: &mut LogServiceClient<Channel>, follow: bool) -> Result<()> {
    logs::handle(client, Some("kernel".to_string()), None, follow, None).await
}
