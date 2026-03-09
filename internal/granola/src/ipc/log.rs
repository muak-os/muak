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

#[cfg(test)]
mod tests {
    use tokio_stream::StreamExt;
    use tonic::Request;

    use super::*;
    use crate::supervisor::logger::{self, LogEntry, LogStream};

    fn make_entry(service: &str, stream: LogStream, message: &str, ts: u64) -> LogEntry {
        LogEntry {
            timestamp_nanos: ts,
            service: service.to_string(),
            stream,
            message: message.to_string(),
        }
    }

    #[test]
    fn to_proto_entry_stdout() {
        // ARRANGE
        let entry = make_entry("svc", LogStream::Stdout, "hello", 12345);

        // ACT
        let proto = to_proto_entry(entry);

        // ASSERT
        assert_eq!(proto.timestamp, 12345);
        assert_eq!(proto.service, "svc");
        assert_eq!(proto.message, "hello");
        assert_eq!(proto.stream, Stream::Stdout as i32);
    }

    #[test]
    fn to_proto_entry_stderr() {
        // ARRANGE
        let entry = make_entry("svc", LogStream::Stderr, "err-line", 99);

        // ACT
        let proto = to_proto_entry(entry);

        // ASSERT
        assert_eq!(proto.stream, Stream::Stderr as i32);
    }

    #[tokio::test]
    async fn get_logs_all_services_no_filter() {
        // ARRANGE
        let (writer, reader, actor) = logger::create();
        tokio::spawn(actor.run());
        writer.append("svc-a", LogStream::Stdout, "msg-a".to_string());
        writer.append("svc-b", LogStream::Stdout, "msg-b".to_string());
        tokio::task::yield_now().await;

        let svc = LogServiceImpl { reader };
        let request = Request::new(GetLogsRequest {
            service: String::new(),
            tail: 0,
        });

        // ACT
        let response = svc.get_logs(request).await.expect("get_logs failed");

        // ASSERT
        let entries = response.into_inner().entries;
        assert_eq!(entries.len(), 2);
    }

    #[tokio::test]
    async fn get_logs_filtered_by_service() {
        // ARRANGE
        let (writer, reader, actor) = logger::create();
        tokio::spawn(actor.run());
        writer.append("svc-a", LogStream::Stdout, "from-a".to_string());
        writer.append("svc-b", LogStream::Stdout, "from-b".to_string());
        tokio::task::yield_now().await;

        let svc = LogServiceImpl { reader };
        let request = Request::new(GetLogsRequest {
            service: "svc-a".to_string(),
            tail: 0,
        });

        // ACT
        let response = svc.get_logs(request).await.expect("get_logs failed");

        // ASSERT
        let entries = response.into_inner().entries;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].service, "svc-a");
        assert_eq!(entries[0].message, "from-a");
    }

    #[tokio::test]
    async fn get_logs_with_tail_limit() {
        // ARRANGE
        let (writer, reader, actor) = logger::create();
        tokio::spawn(actor.run());
        for i in 0..5u32 {
            writer.append("svc", LogStream::Stdout, format!("msg{i}"));
        }
        tokio::task::yield_now().await;

        let svc = LogServiceImpl { reader };
        let request = Request::new(GetLogsRequest {
            service: "svc".to_string(),
            tail: 2,
        });

        // ACT
        let response = svc.get_logs(request).await.expect("get_logs failed");

        // ASSERT
        let entries = response.into_inner().entries;
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].message, "msg3");
        assert_eq!(entries[1].message, "msg4");
    }

    #[tokio::test]
    async fn follow_logs_receives_live_entries_no_filter() {
        // ARRANGE
        let (writer, reader, actor) = logger::create();
        tokio::spawn(actor.run());

        let svc = LogServiceImpl { reader };
        let request = Request::new(FollowLogsRequest {
            service: String::new(),
        });

        // ACT
        let response = svc.follow_logs(request).await.expect("follow_logs failed");
        let mut stream = response.into_inner();

        writer.append("svc", LogStream::Stdout, "live-entry".to_string());

        // ASSERT
        let item = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
            .await
            .expect("timed out")
            .expect("stream ended")
            .expect("stream error");
        assert_eq!(item.message, "live-entry");
    }

    #[tokio::test]
    async fn follow_logs_filters_by_service() {
        // ARRANGE
        let (writer, reader, actor) = logger::create();
        tokio::spawn(actor.run());

        let svc = LogServiceImpl { reader };
        let request = Request::new(FollowLogsRequest {
            service: "wanted".to_string(),
        });

        // ACT
        let response = svc.follow_logs(request).await.expect("follow_logs failed");
        let mut stream = response.into_inner();

        writer.append("other", LogStream::Stdout, "ignored".to_string());
        writer.append("wanted", LogStream::Stdout, "expected".to_string());

        // ASSERT
        let item = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
            .await
            .expect("timed out")
            .expect("stream ended")
            .expect("stream error");
        assert_eq!(item.message, "expected");
        assert_eq!(item.service, "wanted");
    }

    #[tokio::test]
    async fn follow_logs_channel_closed_terminates_stream() {
        // ARRANGE
        let (writer, reader, actor) = logger::create();
        let actor_handle = tokio::spawn(actor.run());

        let svc = LogServiceImpl { reader };
        let request = Request::new(FollowLogsRequest {
            service: String::new(),
        });

        // ACT
        let response = svc.follow_logs(request).await.expect("follow_logs failed");
        let mut stream = response.into_inner();

        drop(writer);
        actor_handle.abort();

        // ASSERT
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next()).await;
        drop(result);
    }
}
