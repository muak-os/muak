//! Generic streaming helper for async tasks that emit progress updates.

use std::future::Future;
use std::pin::Pin;

use tonic::Status;

/// A pinned gRPC server-streaming type for progress messages.
pub type ProgressStream<T> = Pin<Box<dyn tokio_stream::Stream<Item = Result<T, Status>> + Send>>;

/// Runs an async task that emits progress via a sender, returning a gRPC stream.
pub fn run<T, R, F, Fut, C>(task: F, on_complete: C) -> ProgressStream<T>
where
    T: Send + 'static,
    R: Send + 'static,
    F: FnOnce(tokio::sync::mpsc::Sender<T>) -> Fut + Send + 'static,
    Fut: Future<Output = anyhow::Result<R>> + Send + 'static,
    C: FnOnce(anyhow::Result<R>, tokio::sync::mpsc::Sender<Result<T, Status>>) + Send + 'static,
{
    let (out_tx, out_rx) = tokio::sync::mpsc::channel::<Result<T, Status>>(32);

    tokio::spawn(async move {
        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel::<T>(32);

        let forward_tx = out_tx.clone();
        let forward_handle = tokio::spawn(async move {
            while let Some(item) = progress_rx.recv().await {
                if forward_tx.send(Ok(item)).await.is_err() {
                    break;
                }
            }
        });

        let result = task(progress_tx).await;

        let _ = forward_handle.await;

        on_complete(result, out_tx);
    });

    Box::pin(tokio_stream::wrappers::ReceiverStream::new(out_rx))
}

/// Sends a progress message via `tx`, ignoring a dropped receiver.
pub async fn send_progress<T: Send>(tx: &tokio::sync::mpsc::Sender<T>, msg: T) {
    let _ = tx.send(msg).await;
}
