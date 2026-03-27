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

#[cfg(test)]
mod tests {
    use tokio_stream::StreamExt;

    use super::*;

    #[tokio::test]
    async fn send_progress_delivers_message_to_receiver() {
        // ARRANGE
        let (tx, mut rx) = tokio::sync::mpsc::channel::<u32>(4);

        // ACT
        send_progress(&tx, 42u32).await;

        // ASSERT
        let received = rx.recv().await;
        assert_eq!(received, Some(42));
    }

    #[tokio::test]
    async fn send_progress_does_not_panic_on_closed_receiver() {
        // ARRANGE
        let (tx, rx) = tokio::sync::mpsc::channel::<u32>(4);
        drop(rx);

        // ACT & ASSERT
        send_progress(&tx, 1u32).await;
    }

    #[tokio::test]
    async fn run_forwards_progress_items_and_calls_on_complete() {
        // ARRANGE
        let task = |tx: tokio::sync::mpsc::Sender<u32>| async move {
            send_progress(&tx, 1u32).await;
            send_progress(&tx, 2u32).await;
            Ok::<_, anyhow::Error>(99u32)
        };

        let (done_tx, mut done_rx) = tokio::sync::mpsc::channel::<u32>(1);
        let on_complete =
            move |result: anyhow::Result<u32>,
                  _out: tokio::sync::mpsc::Sender<Result<u32, Status>>| {
                let _ = done_tx.try_send(result.unwrap());
            };

        // ACT
        let mut stream = run(task, on_complete);
        let first = stream.next().await;
        let second = stream.next().await;
        while stream.next().await.is_some() {}

        // ASSERT
        assert_eq!(first.and_then(|r| r.ok()), Some(1u32));
        assert_eq!(second.and_then(|r| r.ok()), Some(2u32));
        let completion_value = done_rx.recv().await;
        assert_eq!(completion_value, Some(99u32));
    }
}
