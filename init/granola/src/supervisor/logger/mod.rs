use alloc::collections::VecDeque;
use std::collections::HashMap;
use std::time::Instant;

use tokio::sync::{broadcast, mpsc, oneshot};

pub mod sources;

/// Maximum log entries kept per service.
const MAX_ENTRIES_PER_SERVICE: usize = 2000;

/// Size of the broadcast channel for live followers.
const BROADCAST_CAPACITY: usize = 256;

/// Severity level for a log entry, using syslog priority values as discriminants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum LogLevel {
    Error = 3,
    Warn = 4,
    Info = 6,
    Debug = 7,
}

impl TryFrom<u8> for LogLevel {
    type Error = ();

    fn try_from(number: u8) -> Result<Self, ()> {
        match number {
            0..=3 => Ok(Self::Error),
            4 => Ok(Self::Warn),
            5 | 6 => Ok(Self::Info),
            7 => Ok(Self::Debug),
            _ => Err(()),
        }
    }
}

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
    pub level: LogLevel,
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
    pub fn append(&self, service: &str, stream: LogStream, level: LogLevel, message: String) {
        let entry = LogEntry {
            timestamp_nanos: u64::try_from(self.epoch.elapsed().as_nanos()).unwrap_or(u64::MAX),
            service: service.to_owned(),
            stream,
            level,
            message,
        };
        drop(self.tx.send(LogCommand::Append(entry)));
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
        drop(self.tx.send(LogCommand::Query {
            service,
            tail,
            reply: reply_tx,
        }));

        reply_rx.await.unwrap_or_default()
    }

    /// Subscribes to live log entries.
    pub async fn subscribe(&self) -> Option<broadcast::Receiver<LogEntry>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        drop(self.tx.send(LogCommand::Subscribe { reply: reply_tx }));

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
                drop(reply.send(self.query(service.as_deref(), tail)));
            }
            LogCommand::Subscribe { reply } => {
                drop(reply.send(self.broadcast_tx.subscribe()));
            }
        }
    }

    fn handle_append(&mut self, entry: LogEntry) {
        drop(self.broadcast_tx.send(entry.clone()));

        let ring = self
            .entries
            .entry(entry.service.clone())
            .or_insert_with(|| VecDeque::with_capacity(MAX_ENTRIES_PER_SERVICE));

        if ring.len() >= MAX_ENTRIES_PER_SERVICE {
            ring.pop_front();
        }
        ring.push_back(entry);
    }

    fn query(&self, service: Option<&str>, tail: usize) -> Vec<LogEntry> {
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tokio::sync::{broadcast, mpsc};

    use super::*;
    use crate::supervisor::logger::LogCommand;

    /// Appends a stdout info message for the `svc` test service.
    fn append_message(writer: &LogWriter, message: String) {
        writer.append("svc", LogStream::Stdout, LogLevel::Info, message);
    }

    /// Appends a stdout info message for an arbitrary test service.
    fn append_entry(writer: &LogWriter, service: &str, message: String) {
        writer.append(service, LogStream::Stdout, LogLevel::Info, message);
    }

    /// Fills the ring buffer of service `svc` with `count + 1` messages.
    fn fill_ring(writer: &LogWriter, count: usize) {
        for number in 0..=count {
            writer.append(
                "svc",
                LogStream::Stdout,
                LogLevel::Info,
                format!("msg{number}"),
            );
        }
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

    #[tokio::test]
    async fn append_and_query_single_service() {
        // ARRANGE
        let (writer, reader, actor) = create();
        tokio::spawn(actor.run());

        // ACT
        writer.append("svc", LogStream::Stdout, LogLevel::Info, "hello".to_owned());
        writer.append("svc", LogStream::Stdout, LogLevel::Info, "world".to_owned());

        // ASSERT
        let entries = reader.query(Some("svc".to_owned()), 0).await;
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries.first().map(|entry| entry.message.as_str()),
            Some("hello")
        );
        assert_eq!(
            entries.get(1).map(|entry| entry.message.as_str()),
            Some("world")
        );
    }

    #[tokio::test]
    async fn query_service_with_tail_returns_last_n() {
        // ARRANGE
        let (writer, reader, actor) = create();
        tokio::spawn(actor.run());
        (0..5_u32).for_each(|number| append_message(&writer, format!("msg{number}")));

        // ACT
        let entries = reader.query(Some("svc".to_owned()), 3).await;

        // ASSERT
        assert_eq!(entries.len(), 3);
        assert_eq!(
            entries.first().map(|entry| entry.message.as_str()),
            Some("msg2")
        );
        assert_eq!(
            entries.get(1).map(|entry| entry.message.as_str()),
            Some("msg3")
        );
        assert_eq!(
            entries.get(2).map(|entry| entry.message.as_str()),
            Some("msg4")
        );
    }

    #[tokio::test]
    async fn query_unknown_service_returns_empty() {
        // ARRANGE
        let (_writer, reader, actor) = create();
        tokio::spawn(actor.run());

        // ACT
        let entries = reader.query(Some("no-such-svc".to_owned()), 0).await;

        // ASSERT
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn query_none_service_returns_all_sorted_by_timestamp() {
        // ARRANGE
        let (writer, reader, actor) = create();
        let handle = tokio::spawn(actor.run());

        writer.append(
            "svc-b",
            LogStream::Stdout,
            LogLevel::Info,
            "from-b".to_owned(),
        );
        writer.append(
            "svc-a",
            LogStream::Stdout,
            LogLevel::Info,
            "from-a".to_owned(),
        );

        tokio::task::yield_now().await;

        // ACT
        let entries = reader.query(None, 0).await;
        drop(handle);

        // ASSERT
        assert_eq!(entries.len(), 2);
        let timestamps: Vec<u64> = entries.iter().map(|entry| entry.timestamp_nanos).collect();
        let sorted = timestamps.is_sorted();
        assert!(sorted, "timestamps should be sorted: {timestamps:?}");
    }

    #[tokio::test]
    async fn query_all_with_tail_returns_last_n_across_services() {
        // ARRANGE
        let (writer, reader, actor) = create();
        tokio::spawn(actor.run());

        (0..4_u32).for_each(|number| append_entry(&writer, "svc-a", format!("a{number}")));
        (0..4_u32).for_each(|number| append_entry(&writer, "svc-b", format!("b{number}")));
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
        writer.append("svc", LogStream::Stdout, LogLevel::Info, "msg".to_owned());
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

        fill_ring(&writer, MAX_ENTRIES_PER_SERVICE);
        tokio::task::yield_now().await;

        // ACT
        let entries = reader.query(Some("svc".to_owned()), 0).await;

        // ASSERT
        assert_eq!(entries.len(), MAX_ENTRIES_PER_SERVICE);
        assert_eq!(
            entries.first().map(|entry| entry.message.as_str()),
            Some("msg1")
        );
        assert_eq!(
            entries.last().map(|entry| entry.message.as_str()),
            Some(format!("msg{MAX_ENTRIES_PER_SERVICE}").as_str())
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
        writer.append(
            "svc",
            LogStream::Stderr,
            LogLevel::Error,
            "live-msg".to_owned(),
        );

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
        writer.append("svc", LogStream::Stdout, LogLevel::Info, "out".to_owned());
        writer.append("svc", LogStream::Stderr, LogLevel::Error, "err".to_owned());

        let entries = reader.query(Some("svc".to_owned()), 0).await;

        // ASSERT
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries.first().map(|entry| entry.stream),
            Some(LogStream::Stdout)
        );
        assert_eq!(
            entries.get(1).map(|entry| entry.stream),
            Some(LogStream::Stderr)
        );
    }

    #[test]
    fn query_all_tail_larger_than_total_returns_all() {
        // ARRANGE
        let mut actor = LogActor {
            rx: unbounded_channel_for_test().1,
            entries: HashMap::new(),
            broadcast_tx: broadcast::channel(BROADCAST_CAPACITY).0,
        };
        actor.handle_append(make_entry("svc", LogStream::Stdout, "a", 1));
        actor.handle_append(make_entry("svc", LogStream::Stdout, "b", 2));

        // ACT
        let result = actor.query(None, 100);

        // ASSERT
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn query_service_tail_larger_than_ring_returns_all() {
        // ARRANGE
        let mut actor = LogActor {
            rx: unbounded_channel_for_test().1,
            entries: HashMap::new(),
            broadcast_tx: broadcast::channel(BROADCAST_CAPACITY).0,
        };
        actor.handle_append(make_entry("svc", LogStream::Stdout, "a", 1));
        actor.handle_append(make_entry("svc", LogStream::Stdout, "b", 2));

        // ACT
        let result = actor.query(Some("svc"), 50);

        // ASSERT
        assert_eq!(result.len(), 2);
    }

    /// Creates a throwaway channel for constructing a test log actor.
    fn unbounded_channel_for_test() -> (
        mpsc::UnboundedSender<LogCommand>,
        mpsc::UnboundedReceiver<LogCommand>,
    ) {
        mpsc::unbounded_channel()
    }

    #[test]
    fn log_level_ordering() {
        // ARRANGE & ACT & ASSERT
        assert!(LogLevel::Error < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Debug);
    }

    #[tokio::test]
    async fn append_preserves_log_level() {
        // ARRANGE
        let (writer, reader, actor) = create();
        tokio::spawn(actor.run());
        // ACT
        writer.append("svc", LogStream::Stdout, LogLevel::Debug, "dbg".to_owned());
        writer.append("svc", LogStream::Stdout, LogLevel::Warn, "wrn".to_owned());
        writer.append("svc", LogStream::Stderr, LogLevel::Error, "err".to_owned());
        let entries = reader.query(Some("svc".to_owned()), 0).await;
        // ASSERT
        assert_eq!(entries.len(), 3);
        assert_eq!(
            entries.first().map(|entry| entry.level),
            Some(LogLevel::Debug)
        );
        assert_eq!(
            entries.get(1).map(|entry| entry.level),
            Some(LogLevel::Warn)
        );
        assert_eq!(
            entries.get(2).map(|entry| entry.level),
            Some(LogLevel::Error)
        );
    }
}
