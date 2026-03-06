use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use super::proto::log::log_service_server::{LogService, LogServiceServer};
use super::proto::log::{
    FollowLogsRequest, GetLogsRequest, GetLogsResponse, LogEntry as ProtoLogEntry, Stream,
};
use crate::supervisor::logger::{LogReader, LogStream};

pub fn service(reader: LogReader) -> LogServiceServer<LogServiceImpl> {
    LogServiceServer::new(LogServiceImpl { reader })
}

pub struct LogServiceImpl {
    reader: LogReader,
}

#[tonic::async_trait]
impl LogService for LogServiceImpl {
    async fn get_logs(
        &self,
        request: Request<GetLogsRequest>,
    ) -> Result<Response<GetLogsResponse>, Status> {
        let req = request.into_inner();

        let service = if req.service.is_empty() {
            None
        } else {
            Some(req.service)
        };

        let entries = self.reader.query(service, req.tail as usize).await;

        let proto_entries: Vec<ProtoLogEntry> = entries.into_iter().map(to_proto_entry).collect();

        Ok(Response::new(GetLogsResponse {
            entries: proto_entries,
        }))
    }

    type FollowLogsStream = ReceiverStream<Result<ProtoLogEntry, Status>>;

    async fn follow_logs(
        &self,
        request: Request<FollowLogsRequest>,
    ) -> Result<Response<Self::FollowLogsStream>, Status> {
        let req = request.into_inner();
        let service_filter = if req.service.is_empty() {
            None
        } else {
            Some(req.service)
        };

        let mut broadcast_rx = self
            .reader
            .subscribe()
            .await
            .ok_or_else(|| Status::internal("Failed to subscribe to log stream"))?;

        let (tx, rx) = tokio::sync::mpsc::channel(128);

        tokio::spawn(async move {
            loop {
                match broadcast_rx.recv().await {
                    Ok(entry) => {
                        // Apply service filter if set.
                        if let Some(ref filter) = service_filter
                            && entry.service != *filter
                        {
                            continue;
                        }
                        if tx.send(Ok(to_proto_entry(entry))).await.is_err() {
                            break; // Client disconnected.
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        kmsg::warn!("Log follower lagged, skipped {} entries", n);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }
}

fn to_proto_entry(entry: crate::supervisor::logger::LogEntry) -> ProtoLogEntry {
    ProtoLogEntry {
        timestamp: entry.timestamp_nanos,
        service: entry.service,
        stream: match entry.stream {
            LogStream::Stdout => Stream::Stdout.into(),
            LogStream::Stderr => Stream::Stderr.into(),
        },
        message: entry.message,
    }
}
