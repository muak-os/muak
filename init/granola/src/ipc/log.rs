use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use super::proto::log::log_service_server::{LogService, LogServiceServer};
use super::proto::log::{
    FollowLogsRequest, GetLogsRequest, GetLogsResponse, Level, LogEntry as ProtoLogEntry, Stream,
};
use crate::supervisor::logger::{LogEntry, LogLevel, LogReader, LogStream};

pub fn service(reader: LogReader) -> LogServiceServer<ServiceImpl> {
    LogServiceServer::new(ServiceImpl { reader })
}

pub struct ServiceImpl {
    reader: LogReader,
}

#[tonic::async_trait]
impl LogService for ServiceImpl {
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

        let entries = self
            .reader
            .query(service, usize::try_from(req.tail).unwrap_or_default())
            .await;

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

        let (tx, rx) = mpsc::channel(128);
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
                    Err(RecvError::Lagged(count)) => {
                        kmsg::warn!("Log follower lagged, skipped {count} entries");
                    }
                    Err(RecvError::Closed) => {
                        break;
                    }
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }
}

fn to_proto_entry(entry: LogEntry) -> ProtoLogEntry {
    ProtoLogEntry {
        timestamp: entry.timestamp_nanos,
        service: entry.service,
        stream: match entry.stream {
            LogStream::Stdout => Stream::Stdout.into(),
            LogStream::Stderr => Stream::Stderr.into(),
        },
        level: match entry.level {
            LogLevel::Error => Level::Error.into(),
            LogLevel::Warn => Level::Warn.into(),
            LogLevel::Info => Level::Info.into(),
            LogLevel::Debug => Level::Debug.into(),
        },
        message: entry.message,
    }
}

#[cfg(test)]
mod tests {
    use tokio_stream::StreamExt as _;
    use tonic::Request;

    use super::*;
    use crate::supervisor::logger::{self, LogEntry, LogLevel, LogStream};

    /// Appends a stdout info message for the `svc` test service.
    fn append_message(writer: &logger::LogWriter, message: String) {
        writer.append("svc", LogStream::Stdout, LogLevel::Info, message);
    }

    fn make_entry(service: &str, stream: LogStream, message: &str, ts: u64) -> LogEntry {
        LogEntry {
            timestamp_nanos: ts,
            service: service.to_owned(),
            stream,
            level: LogLevel::Info,
            message: message.to_owned(),
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
        assert_eq!(proto.stream, i32::from(Stream::Stdout));
        assert_eq!(proto.level, i32::from(Level::Info));
    }

    #[test]
    fn to_proto_entry_stderr() {
        // ARRANGE
        let entry = make_entry("svc", LogStream::Stderr, "err-line", 99);

        // ACT
        let proto = to_proto_entry(entry);

        // ASSERT
        assert_eq!(proto.stream, i32::from(Stream::Stderr));
    }

    #[test]
    fn to_proto_entry_maps_all_levels() {
        // ARRANGE
        let levels = [
            (LogLevel::Error, Level::Error),
            (LogLevel::Warn, Level::Warn),
            (LogLevel::Info, Level::Info),
            (LogLevel::Debug, Level::Debug),
        ];

        for (internal, expected_proto) in levels {
            // ACT
            let entry = LogEntry {
                timestamp_nanos: 0,
                service: "svc".to_owned(),
                stream: LogStream::Stdout,
                level: internal,
                message: String::new(),
            };
            let proto = to_proto_entry(entry);

            // ASSERT
            assert_eq!(proto.level, i32::from(expected_proto));
        }
    }

    #[tokio::test]
    async fn get_logs_all_services_no_filter() {
        // ARRANGE
        let (writer, reader, actor) = logger::create();
        tokio::spawn(actor.run());
        writer.append(
            "svc-a",
            LogStream::Stdout,
            LogLevel::Info,
            "msg-a".to_owned(),
        );
        writer.append(
            "svc-b",
            LogStream::Stdout,
            LogLevel::Info,
            "msg-b".to_owned(),
        );
        tokio::task::yield_now().await;

        let svc = ServiceImpl { reader };
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
        writer.append(
            "svc-a",
            LogStream::Stdout,
            LogLevel::Info,
            "from-a".to_owned(),
        );
        writer.append(
            "svc-b",
            LogStream::Stdout,
            LogLevel::Info,
            "from-b".to_owned(),
        );
        tokio::task::yield_now().await;

        let svc = ServiceImpl { reader };
        let request = Request::new(GetLogsRequest {
            service: "svc-a".to_owned(),
            tail: 0,
        });

        // ACT
        let response = svc.get_logs(request).await.expect("get_logs failed");

        // ASSERT
        let entries = response.into_inner().entries;
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries.first().map(|entry| entry.service.as_str()),
            Some("svc-a")
        );
        assert_eq!(
            entries.first().map(|entry| entry.message.as_str()),
            Some("from-a")
        );
    }

    #[tokio::test]
    async fn get_logs_with_tail_limit() {
        // ARRANGE
        let (writer, reader, actor) = logger::create();
        tokio::spawn(actor.run());
        (0..5_u32).for_each(|number| append_message(&writer, format!("msg{number}")));
        tokio::task::yield_now().await;

        let svc = ServiceImpl { reader };
        let request = Request::new(GetLogsRequest {
            service: "svc".to_owned(),
            tail: 2,
        });

        // ACT
        let response = svc.get_logs(request).await.expect("get_logs failed");

        // ASSERT
        let entries = response.into_inner().entries;
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries.first().map(|entry| entry.message.as_str()),
            Some("msg3")
        );
        assert_eq!(
            entries.get(1).map(|entry| entry.message.as_str()),
            Some("msg4")
        );
    }

    #[tokio::test]
    async fn follow_logs_receives_live_entries_no_filter() {
        // ARRANGE
        let (writer, reader, actor) = logger::create();
        tokio::spawn(actor.run());

        let svc = ServiceImpl { reader };
        let request = Request::new(FollowLogsRequest {
            service: String::new(),
        });

        // ACT
        let response = svc.follow_logs(request).await.expect("follow_logs failed");
        let mut stream = response.into_inner();

        writer.append(
            "svc",
            LogStream::Stdout,
            LogLevel::Info,
            "live-entry".to_owned(),
        );

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

        let svc = ServiceImpl { reader };
        let request = Request::new(FollowLogsRequest {
            service: "wanted".to_owned(),
        });

        // ACT
        let response = svc.follow_logs(request).await.expect("follow_logs failed");
        let mut stream = response.into_inner();

        writer.append(
            "other",
            LogStream::Stdout,
            LogLevel::Info,
            "ignored".to_owned(),
        );
        writer.append(
            "wanted",
            LogStream::Stdout,
            LogLevel::Info,
            "expected".to_owned(),
        );

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

        let svc = ServiceImpl { reader };
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
