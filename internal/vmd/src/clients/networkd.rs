use anyhow::Result;
use hyper_util::rt::TokioIo;
use std::path::Path;
use tokio::net::UnixStream;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

use crate::proto::network::network_service_client::NetworkServiceClient;
use crate::proto::network::{CreateTapRequest, DeleteTapRequest};

#[derive(Clone)]
pub struct NetworkClient {
    client: NetworkServiceClient<Channel>,
}

impl NetworkClient {
    pub async fn connect(socket_path: &str) -> Result<Self> {
        let socket_path = socket_path.to_string();

        if !Path::new(&socket_path).exists() {
            anyhow::bail!("networkd socket not found at {}", socket_path);
        }

        let channel = Endpoint::try_from("http://[::]:50051")?
            .connect_with_connector(service_fn(move |_: Uri| {
                let path = socket_path.clone();
                async move {
                    let stream = UnixStream::connect(path).await?;
                    Ok::<_, std::io::Error>(TokioIo::new(stream))
                }
            }))
            .await?;

        Ok(Self {
            client: NetworkServiceClient::new(channel),
        })
    }

    pub async fn create_tap(&self, vm_id: &str, name: Option<&str>) -> Result<TapDevice> {
        let mut client = self.client.clone();

        let response = client
            .create_tap(CreateTapRequest {
                vm_id: vm_id.to_string(),
                name: name.map(|s| s.to_string()).unwrap_or_default(),
            })
            .await?
            .into_inner();

        Ok(TapDevice {
            name: response.interface_name,
            mac_address: response.mac_address,
        })
    }

    pub async fn delete_tap(&self, name: &str) -> Result<()> {
        let mut client = self.client.clone();

        client
            .delete_tap(DeleteTapRequest {
                name: name.to_string(),
            })
            .await?;

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct TapDevice {
    pub name: String,
    pub mac_address: String,
}
