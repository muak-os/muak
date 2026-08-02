//! Generic streaming helper for async tasks that emit progress updates.

use core::future::Future;
use core::pin::Pin;

use tokio::sync::mpsc;
use tonic::Status;

/// A pinned gRPC server-streaming type for progress messages.
pub type ProgressStream<T> = Pin<Box<dyn tokio_stream::Stream<Item = Result<T, Status>> + Send>>;

/// Runs an async task that emits progress via a sender, returning a gRPC stream.
pub fn run<T, R, F, Fut, C>(task: F, on_complete: C) -> ProgressStream<T>
where
    T: Send + 'static,
    R: Send + 'static,
    F: FnOnce(mpsc::Sender<T>) -> Fut + Send + 'static,
    Fut: Future<Output = anyhow::Result<R>> + Send + 'static,
    C: FnOnce(anyhow::Result<R>, mpsc::Sender<Result<T, Status>>) + Send + 'static,
{
    let (out_tx, out_rx) = mpsc::channel::<Result<T, Status>>(32);

    tokio::spawn(async move {
        let (progress_tx, progress_rx) = mpsc::channel::<T>(32);

        let forward_tx = out_tx.clone();
        let forward_handle = tokio::spawn(forward_progress(progress_rx, forward_tx));

        let result = task(progress_tx).await;

        let _forward_result = forward_handle.await;

        on_complete(result, out_tx);
    });

    Box::pin(tokio_stream::wrappers::ReceiverStream::new(out_rx))
}

async fn forward_progress<T>(
    mut progress_rx: mpsc::Receiver<T>,
    forward_tx: mpsc::Sender<Result<T, Status>>,
) {
    while let Some(item) = progress_rx.recv().await {
        if forward_tx.send(Ok(item)).await.is_err() {
            break;
        }
    }
}

/// Sends a progress message via `tx`, ignoring a dropped receiver.
pub async fn send_progress<T: Send>(tx: &mpsc::Sender<T>, msg: T) {
    let _sent = tx.send(msg).await;
}

#[cfg(test)]
mod tests {
    use tokio_stream::StreamExt as _;

    use super::*;

    async fn drain_rest(stream: &mut ProgressStream<u32>) {
        while stream.next().await.is_some() {}
    }

    async fn send_two_messages(tx: mpsc::Sender<u32>) -> anyhow::Result<u32> {
        send_progress(&tx, 1_u32).await;
        send_progress(&tx, 2_u32).await;
        Ok(99_u32)
    }

    fn capture_completion(done_tx: &mpsc::Sender<u32>, result: anyhow::Result<u32>) {
        let _sent = done_tx.try_send(result.unwrap());
    }

    async fn run_forwards_progress_scenario() -> (
        Option<Result<u32, Status>>,
        Option<Result<u32, Status>>,
        Option<u32>,
    ) {
        let task = send_two_messages;

        let (done_tx, mut done_rx) = mpsc::channel::<u32>(1);
        let on_complete = move |result: anyhow::Result<u32>,
                                _out: mpsc::Sender<Result<u32, Status>>| {
            capture_completion(&done_tx, result);
        };

        let mut stream = run(task, on_complete);
        let first = stream.next().await;
        let second = stream.next().await;
        drain_rest(&mut stream).await;

        (first, second, done_rx.recv().await)
    }

    #[tokio::test]
    async fn send_progress_delivers_message_to_receiver() {
        // ARRANGE
        let (tx, mut rx) = mpsc::channel::<u32>(4);

        // ACT
        send_progress(&tx, 42_u32).await;

        // ASSERT
        let received = rx.recv().await;
        assert_eq!(received, Some(42));
    }

    #[tokio::test]
    async fn send_progress_does_not_panic_on_closed_receiver() {
        // ARRANGE
        let (tx, rx) = mpsc::channel::<u32>(4);
        drop(rx);

        // ACT & ASSERT
        send_progress(&tx, 1_u32).await;
    }

    #[tokio::test]
    async fn run_forwards_progress_items_and_calls_on_complete() {
        // ARRANGE / ACT
        let (first, second, completion_value) = run_forwards_progress_scenario().await;

        // ASSERT
        assert_eq!(first.and_then(core::result::Result::ok), Some(1_u32));
        assert_eq!(second.and_then(core::result::Result::ok), Some(2_u32));
        assert_eq!(completion_value, Some(99_u32));
    }
}
