//! Unix transport for the separately confined local publisher.

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use std::io;
use std::path::PathBuf;

use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;
use tonic::codegen::Service;
use tonic::transport::Uri;

/// Connector whose destination cannot be supplied by an RPC caller.
#[derive(Clone)]
pub struct Connector(pub PathBuf);

impl Service<Uri> for Connector {
    type Error = io::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;
    type Response = TokioIo<UnixStream>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _uri: Uri) -> Self::Future {
        let path = self.0.clone();
        Box::pin(async move { Ok(TokioIo::new(UnixStream::connect(path).await?)) })
    }
}
