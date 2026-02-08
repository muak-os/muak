//! File upload functionality for VM images and kernels.

use anyhow::Result;
use tokio::io::AsyncReadExt;
use tonic::transport::Channel;

use super::vm_service::{UploadFileMetadata, upload_file_request};
use super::{UploadFileRequest, VmServiceClient};

/// Uploads a file to the server using streaming.
pub async fn upload_file(
    client: &mut VmServiceClient<Channel>,
    file_path: &str,
    vm_id: Option<&str>,
    target_filename: Option<&str>,
) -> Result<String> {
    let mut file = tokio::fs::File::open(file_path).await?;
    let metadata = file.metadata().await?;
    let file_size = metadata.len();

    let filename = target_filename.map(|s| s.to_string()).unwrap_or_else(|| {
        std::path::Path::new(file_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string()
    });

    let vm_id_str = vm_id.unwrap_or("").to_string();

    let (tx, rx) = tokio::sync::mpsc::channel(128);

    tokio::spawn(async move {
        let metadata_msg = UploadFileRequest {
            request: Some(upload_file_request::Request::Metadata(UploadFileMetadata {
                filename,
                size: file_size as i64,
                vm_id: vm_id_str,
            })),
        };

        if tx.send(metadata_msg).await.is_err() {
            return;
        }

        let mut buffer = vec![0u8; 1024 * 1024];
        loop {
            let n = match file.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };

            let chunk = UploadFileRequest {
                request: Some(upload_file_request::Request::Chunk(buffer[..n].to_vec())),
            };
            if tx.send(chunk).await.is_err() {
                break;
            }
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    let request = tonic::Request::new(stream);
    let response = client.upload_file(request).await?;
    let resp = response.into_inner();

    if !resp.error.is_empty() {
        return Err(anyhow::anyhow!("Upload failed: {}", resp.error));
    }

    Ok(resp.path)
}
