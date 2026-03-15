use std::collections::{HashMap, VecDeque};
use std::os::fd::OwnedFd;
use std::time::Instant;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{broadcast, mpsc, oneshot};

/// Maximum log entries kept per service.
const MAX_ENTRIES_PER_SERVICE: usize = 2000;

/// Size of the broadcast channel for live followers.
const BROADCAST_CAPACITY: usize = 256;

/// Which output stream a log line came from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LogStream {
    Stdout,
    Stderr,
}

/// A single captured log line.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp_nanos: u64,
    pub service: String,
    pub stream: LogStream,
    pub message: String,
}

/// Commands sent to the log actor.
enum LogCommand {
    Append(LogEntry),
    Query {
        service: Option<String>,
        tail: usize,
        reply: oneshot::Sender<Vec<LogEntry>>,
    },
    Subscribe {
        reply: oneshot::Sender<broadcast::Receiver<LogEntry>>,
    },
}

/// Provides log appending capabilities to services.
#[derive(Clone)]
pub struct LogWriter {
    tx: mpsc::UnboundedSender<LogCommand>,
    epoch: Instant,
}

impl LogWriter {
    /// Sends a log entry to the actor.
    pub fn append(&self, service: &str, stream: LogStream, message: String) {
        let entry = LogEntry {
            timestamp_nanos: self.epoch.elapsed().as_nanos() as u64,
            service: service.to_string(),
            stream,
            message,
        };
        let _ = self.tx.send(LogCommand::Append(entry));
    }
}

/// Provides log querying and live subscription capabilities.
#[derive(Clone)]
pub struct LogReader {
    tx: mpsc::UnboundedSender<LogCommand>,
}

impl LogReader {
    /// Query stored logs.
    pub async fn query(&self, service: Option<String>, tail: usize) -> Vec<LogEntry> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let _ = self.tx.send(LogCommand::Query {
            service,
            tail,
            reply: reply_tx,
        });
        reply_rx.await.unwrap_or_default()
    }

    /// Subscribes to live log entries.
    pub async fn subscribe(&self) -> Option<broadcast::Receiver<LogEntry>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let _ = self.tx.send(LogCommand::Subscribe { reply: reply_tx });
        reply_rx.await.ok()
    }
}

/// The log storage actor. Owns all log data, no shared mutable state.
pub struct LogActor {
    rx: mpsc::UnboundedReceiver<LogCommand>,
    entries: HashMap<String, VecDeque<LogEntry>>,
    broadcast_tx: broadcast::Sender<LogEntry>,
}

impl LogActor {
    /// Runs the actor loop until all senders are dropped.
    pub async fn run(mut self) {
        while let Some(cmd) = self.rx.recv().await {
            self.handle(cmd);
        }
    }

    fn handle(&mut self, cmd: LogCommand) {
        match cmd {
            LogCommand::Append(entry) => self.handle_append(entry),
            LogCommand::Query {
                service,
                tail,
                reply,
            } => {
                let _ = reply.send(self.query(&service, tail));
            }
            LogCommand::Subscribe { reply } => {
                let _ = reply.send(self.broadcast_tx.subscribe());
            }
        }
    }

    fn handle_append(&mut self, entry: LogEntry) {
        let _ = self.broadcast_tx.send(entry.clone());

        let ring = self
            .entries
            .entry(entry.service.clone())
            .or_insert_with(|| VecDeque::with_capacity(MAX_ENTRIES_PER_SERVICE));

        if ring.len() >= MAX_ENTRIES_PER_SERVICE {
            ring.pop_front();
        }
        ring.push_back(entry);
    }

    fn query(&self, service: &Option<String>, tail: usize) -> Vec<LogEntry> {
        match service {
            Some(name) if !name.is_empty() => self.query_service(name, tail),
            _ => self.query_all(tail),
        }
    }

    fn query_service(&self, name: &str, tail: usize) -> Vec<LogEntry> {
        let Some(ring) = self.entries.get(name) else {
            return Vec::new();
        };
        if tail == 0 {
            return ring.iter().cloned().collect();
        }
        ring.iter().rev().take(tail).rev().cloned().collect()
    }

    fn query_all(&self, tail: usize) -> Vec<LogEntry> {
        let mut all: Vec<LogEntry> = self
            .entries
            .values()
            .flat_map(|ring| ring.iter().cloned())
            .collect();
        all.sort_by_key(|e| e.timestamp_nanos);

        if tail == 0 {
            return all;
        }
        let skip = all.len().saturating_sub(tail);
        all.into_iter().skip(skip).collect()
    }
}

/// Creates the log writer, reader, and actor.
pub fn create() -> (LogWriter, LogReader, LogActor) {
    let (tx, rx) = mpsc::unbounded_channel();
    let epoch = Instant::now();

    let writer = LogWriter {
        tx: tx.clone(),
        epoch,
    };
    let reader = LogReader { tx };
    let actor = LogActor {
        rx,
        entries: HashMap::new(),
        broadcast_tx: broadcast::channel(BROADCAST_CAPACITY).0,
    };

    (writer, reader, actor)
}

/// Captures logs from the given stdout and stderr file descriptors, forwards them to the logger.
pub fn capture(name: &str, stdout_fd: OwnedFd, stderr_fd: OwnedFd, logger: &LogWriter) {
    spawn_reader(
        name.to_string(),
        stdout_fd,
        LogStream::Stdout,
        logger.clone(),
    );
    spawn_reader(
        name.to_string(),
        stderr_fd,
        LogStream::Stderr,
        logger.clone(),
    );
}

fn spawn_reader(name: String, fd: OwnedFd, stream: LogStream, logger: LogWriter) {
    tokio::spawn(async move {
        let async_fd = tokio::fs::File::from_std(std::fs::File::from(fd));
        let reader = BufReader::new(async_fd);
        let mut lines = reader.lines();

        while let Ok(Some(line)) = lines.next_line().await {
            logger.append(&name, stream, line);
        }
    });
}

/// Reads kernel messages from `/dev/kmsg` and feeds them into the log actor as `service = "kernel"`.
pub fn kmsg(logger: &LogWriter) {
    let logger = logger.clone();
    tokio::spawn(async move {
        let file = match tokio::fs::OpenOptions::new()
            .read(true)
            .open("/dev/kmsg")
            .await
        {
            Ok(f) => f,
            Err(e) => {
                kmsg::warn!("Failed to open /dev/kmsg for log capture: {}", e);
                return;
            }
        };

        let reader = BufReader::new(file);
        let mut lines = reader.lines();

        while let Ok(Some(line)) = lines.next_line().await {
            let message = if let Some(idx) = line.find(';') {
                line[idx + 1..].to_string()
            } else {
                line
            };
            logger.append("kernel", LogStream::Stdout, message);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(service: &str, stream: LogStream, message: &str, ts: u64) -> LogEntry {
        LogEntry {
            timestamp_nanos: ts,
            service: service.to_string(),
            stream,
            message: message.to_string(),
        }
    }

    #[tokio::test]
    async fn append_and_query_single_service() {
        // ARRANGE
        let (writer, reader, actor) = create();
        tokio::spawn(actor.run());

        // ACT
        writer.append("svc", LogStream::Stdout, "hello".to_string());
        writer.append("svc", LogStream::Stdout, "world".to_string());

        // ASSERT
        let entries = reader.query(Some("svc".to_string()), 0).await;
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].message, "hello");
        assert_eq!(entries[1].message, "world");
    }

    #[tokio::test]
    async fn query_service_with_tail_returns_last_n() {
        // ARRANGE
        let (writer, reader, actor) = create();
        tokio::spawn(actor.run());
        for i in 0..5u32 {
            writer.append("svc", LogStream::Stdout, format!("msg{i}"));
        }

        // ACT
        let entries = reader.query(Some("svc".to_string()), 3).await;

        // ASSERT
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].message, "msg2");
        assert_eq!(entries[1].message, "msg3");
        assert_eq!(entries[2].message, "msg4");
    }

    #[tokio::test]
    async fn query_unknown_service_returns_empty() {
        // ARRANGE
        let (_writer, reader, actor) = create();
        tokio::spawn(actor.run());

        // ACT
        let entries = reader.query(Some("no-such-svc".to_string()), 0).await;

        // ASSERT
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn query_none_service_returns_all_sorted_by_timestamp() {
        // ARRANGE
        let (writer, reader, actor) = create();
        let handle = tokio::spawn(actor.run());

        writer.append("svc-b", LogStream::Stdout, "from-b".to_string());
        writer.append("svc-a", LogStream::Stdout, "from-a".to_string());

        tokio::task::yield_now().await;

        // ACT
        let entries = reader.query(None, 0).await;
        drop(handle);

        // ASSERT
        assert_eq!(entries.len(), 2);
        assert!(entries[0].timestamp_nanos <= entries[1].timestamp_nanos);
    }

    #[tokio::test]
    async fn query_all_with_tail_returns_last_n_across_services() {
        // ARRANGE
        let (writer, reader, actor) = create();
        tokio::spawn(actor.run());

        for i in 0..4u32 {
            writer.append("svc-a", LogStream::Stdout, format!("a{i}"));
        }
        for i in 0..4u32 {
            writer.append("svc-b", LogStream::Stdout, format!("b{i}"));
        }
        tokio::task::yield_now().await;

        // ACT
        let entries = reader.query(None, 3).await;

        // ASSERT
        assert_eq!(entries.len(), 3);
    }

    #[tokio::test]
    async fn query_empty_string_service_returns_all() {
        // ARRANGE
        let (writer, reader, actor) = create();
        tokio::spawn(actor.run());
        writer.append("svc", LogStream::Stdout, "msg".to_string());
        tokio::task::yield_now().await;

        // ACT
        let entries = reader.query(Some(String::new()), 0).await;

        // ASSERT
        assert_eq!(entries.len(), 1);
    }

    #[tokio::test]
    async fn ring_buffer_evicts_oldest_entry_at_capacity() {
        // ARRANGE
        let (writer, reader, actor) = create();
        tokio::spawn(actor.run());

        for i in 0..=MAX_ENTRIES_PER_SERVICE {
            writer.append("svc", LogStream::Stdout, format!("msg{i}"));
        }
        tokio::task::yield_now().await;

        // ACT
        let entries = reader.query(Some("svc".to_string()), 0).await;

        // ASSERT
        assert_eq!(entries.len(), MAX_ENTRIES_PER_SERVICE);
        assert_eq!(entries[0].message, "msg1");
        assert_eq!(
            entries[MAX_ENTRIES_PER_SERVICE - 1].message,
            format!("msg{MAX_ENTRIES_PER_SERVICE}")
        );
    }

    #[tokio::test]
    async fn actor_exits_when_all_senders_dropped() {
        // ARRANGE
        let (writer, reader, actor) = create();
        let handle = tokio::spawn(actor.run());

        // ACT
        drop(writer);
        drop(reader);

        // ASSERT
        tokio::time::timeout(std::time::Duration::from_secs(1), handle)
            .await
            .expect("actor did not exit within timeout")
            .expect("actor task panicked");
    }

    #[tokio::test]
    async fn subscribe_receives_live_entries() {
        // ARRANGE
        let (writer, reader, actor) = create();
        tokio::spawn(actor.run());

        // ACT
        let mut rx = reader.subscribe().await.expect("subscribe failed");
        writer.append("svc", LogStream::Stderr, "live-msg".to_string());

        // ASSERT
        let entry = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .expect("timed out waiting for broadcast entry")
            .expect("broadcast channel closed");
        assert_eq!(entry.message, "live-msg");
        assert_eq!(entry.stream, LogStream::Stderr);
    }

    #[tokio::test]
    async fn log_entry_stream_field_preserved() {
        // ARRANGE
        let (writer, reader, actor) = create();
        tokio::spawn(actor.run());

        // ACT
        writer.append("svc", LogStream::Stdout, "out".to_string());
        writer.append("svc", LogStream::Stderr, "err".to_string());

        let entries = reader.query(Some("svc".to_string()), 0).await;

        // ASSERT
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].stream, LogStream::Stdout);
        assert_eq!(entries[1].stream, LogStream::Stderr);
    }

    #[test]
    fn query_all_tail_larger_than_total_returns_all() {
        // ARRANGE
        let mut actor = LogActor {
            rx: tokio::sync::mpsc::unbounded_channel().1,
            entries: HashMap::new(),
            broadcast_tx: broadcast::channel(BROADCAST_CAPACITY).0,
        };
        actor.handle_append(make_entry("svc", LogStream::Stdout, "a", 1));
        actor.handle_append(make_entry("svc", LogStream::Stdout, "b", 2));

        // ACT - tail larger than total entries; saturating_sub handles gracefully
        let result = actor.query(&None, 100);

        // ASSERT
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn query_service_tail_larger_than_ring_returns_all() {
        // ARRANGE
        let mut actor = LogActor {
            rx: tokio::sync::mpsc::unbounded_channel().1,
            entries: HashMap::new(),
            broadcast_tx: broadcast::channel(BROADCAST_CAPACITY).0,
        };
        actor.handle_append(make_entry("svc", LogStream::Stdout, "a", 1));
        actor.handle_append(make_entry("svc", LogStream::Stdout, "b", 2));

        // ACT
        let result = actor.query(&Some("svc".to_string()), 50);

        // ASSERT
        assert_eq!(result.len(), 2);
    }
}
