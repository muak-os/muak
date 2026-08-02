//! File upload functionality for VM images and kernels.

use std::path::Path;

use anyhow::Result;
use tokio::fs::File;
use tokio::io::AsyncReadExt as _;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::Channel;

use super::vm_service::{
    UploadFileMetadata, UploadFileRequest, upload_file_request, vm_service_client::VmServiceClient,
};

const CHUNK_SIZE: usize = 1024 * 1024;

/// Uploads a file to the server using streaming.
///
/// # Errors
///
/// Returns an error if the file cannot be opened or read, the streaming upload
/// fails, or the server reports an error.
pub async fn upload(
    client: &mut VmServiceClient<Channel>,
    file_path: &str,
    vm_id: Option<&str>,
    target_filename: Option<&str>,
) -> Result<String> {
    let file = File::open(file_path).await?;
    let metadata = file.metadata().await?;
    let file_size = metadata.len();

    let filename = target_filename.map_or_else(
        || {
            Path::new(file_path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("file")
                .to_owned()
        },
        str::to_owned,
    );

    let vm_id_str = vm_id.unwrap_or("").to_owned();

    let (tx, rx) = mpsc::channel(128);

    tokio::spawn(async move {
        let metadata_msg = UploadFileRequest {
            request: Some(upload_file_request::Request::Metadata(UploadFileMetadata {
                filename,
                size: i64::try_from(file_size).unwrap_or(i64::MAX),
                vm_id: vm_id_str,
            })),
        };

        if tx.send(metadata_msg).await.is_err() {
            return;
        }

        stream_chunks(file, tx).await;
    });

    let stream = ReceiverStream::new(rx);
    let request = tonic::Request::new(stream);
    let response = client.upload_file(request).await?;
    let resp = response.into_inner();

    if !resp.error.is_empty() {
        return Err(anyhow::anyhow!("Upload failed: {}", resp.error));
    }

    Ok(resp.path)
}

/// Reads the file in chunks and streams them over the channel.
async fn stream_chunks(mut file: File, tx: mpsc::Sender<UploadFileRequest>) {
    let mut buffer = vec![0; CHUNK_SIZE];
    loop {
        let n = match file.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };

        let chunk = UploadFileRequest {
            request: Some(upload_file_request::Request::Chunk(
                buffer.get(..n).unwrap_or_default().to_vec(),
            )),
        };
        if tx.send(chunk).await.is_err() {
            break;
        }
    }
}
