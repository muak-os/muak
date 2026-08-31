use std::collections::{HashMap, VecDeque};
use std::os::fd::OwnedFd;
use std::time::Instant;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{broadcast, mpsc, oneshot};

/// Maximum log entries kept per service.
const MAX_ENTRIES_PER_SERVICE: usize = 2000;

/// Size of the broadcast channel for live followers.
const BROADCAST_CAPACITY: usize = 256;

/// Severity level for a log entry, using syslog priority values as discriminants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LogLevel {
    Error = 3,
    Warn = 4,
    Info = 6,
    Debug = 7,
}

impl TryFrom<u8> for LogLevel {
    type Error = ();

    fn try_from(n: u8) -> Result<Self, ()> {
        match n {
            0..=3 => Ok(Self::Error),
            4 => Ok(Self::Warn),
            5 | 6 => Ok(Self::Info),
            7 => Ok(Self::Debug),
            _ => Err(()),
        }
    }
}

impl PartialOrd for LogLevel {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for LogLevel {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (*self as u8).cmp(&(*other as u8))
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
            timestamp_nanos: self.epoch.elapsed().as_nanos() as u64,
            service: service.to_string(),
            stream,
            level,
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

/// Extracts a kernel-style syslog priority prefix from a log line.
fn parse_level_prefix(line: &str, default: LogLevel) -> (LogLevel, &str) {
    let Some(rest) = line.strip_prefix('<') else {
        return (default, line);
    };
    let Some(close) = rest.find('>') else {
        return (default, line);
    };
    let Ok(number) = rest[..close].parse::<u8>() else {
        return (default, line);
    };
    let level = LogLevel::try_from(number).unwrap_or(default);
    (level, &rest[close + 1..])
}

fn spawn_reader(name: String, fd: OwnedFd, stream: LogStream, logger: LogWriter) {
    let default_level = match stream {
        LogStream::Stdout => LogLevel::Info,
        LogStream::Stderr => LogLevel::Error,
    };
    tokio::spawn(async move {
        let async_fd = tokio::fs::File::from_std(std::fs::File::from(fd));
        let reader = BufReader::new(async_fd);
        let mut lines = reader.lines();

        while let Ok(Some(line)) = lines.next_line().await {
            let (level, message) = parse_level_prefix(&line, default_level);
            logger.append(&name, stream, level, message.to_string());
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
            logger.append("kernel", LogStream::Stdout, LogLevel::Info, message);
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
            level: LogLevel::Info,
            message: message.to_string(),
        }
    }

    #[tokio::test]
    async fn append_and_query_single_service() {
        // ARRANGE
        let (writer, reader, actor) = create();
        tokio::spawn(actor.run());

        // ACT
        writer.append(
            "svc",
            LogStream::Stdout,
            LogLevel::Info,
            "hello".to_string(),
        );
        writer.append(
            "svc",
            LogStream::Stdout,
            LogLevel::Info,
            "world".to_string(),
        );

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
            writer.append("svc", LogStream::Stdout, LogLevel::Info, format!("msg{i}"));
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

        writer.append(
            "svc-b",
            LogStream::Stdout,
            LogLevel::Info,
            "from-b".to_string(),
        );
        writer.append(
            "svc-a",
            LogStream::Stdout,
            LogLevel::Info,
            "from-a".to_string(),
        );

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
            writer.append("svc-a", LogStream::Stdout, LogLevel::Info, format!("a{i}"));
        }
        for i in 0..4u32 {
            writer.append("svc-b", LogStream::Stdout, LogLevel::Info, format!("b{i}"));
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
        writer.append("svc", LogStream::Stdout, LogLevel::Info, "msg".to_string());
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
            writer.append("svc", LogStream::Stdout, LogLevel::Info, format!("msg{i}"));
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
        writer.append(
            "svc",
            LogStream::Stderr,
            LogLevel::Error,
            "live-msg".to_string(),
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
        writer.append("svc", LogStream::Stdout, LogLevel::Info, "out".to_string());
        writer.append("svc", LogStream::Stderr, LogLevel::Error, "err".to_string());

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

        // ACT
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

    #[test]
    fn parse_level_prefix_extracts_debug() {
        // ARRANGE
        let line = "<7>some debug info";

        // ACT
        let (level, message) = parse_level_prefix(line, LogLevel::Info);

        // ASSERT
        assert_eq!(level, LogLevel::Debug);
        assert_eq!(message, "some debug info");
    }

    #[test]
    fn parse_level_prefix_extracts_warn() {
        // ARRANGE
        let line = "<4>something concerning";

        // ACT
        let (level, message) = parse_level_prefix(line, LogLevel::Info);

        // ASSERT
        assert_eq!(level, LogLevel::Warn);
        assert_eq!(message, "something concerning");
    }

    #[test]
    fn parse_level_prefix_extracts_error() {
        // ARRANGE
        let line = "<3>bad thing happened";

        // ACT
        let (level, message) = parse_level_prefix(line, LogLevel::Info);

        // ASSERT
        assert_eq!(level, LogLevel::Error);
        assert_eq!(message, "bad thing happened");
    }

    #[test]
    fn parse_level_prefix_high_severity_maps_to_error() {
        // ARRANGE
        for n in 0u8..=2 {
            let line = format!("<{n}>critical message");

            // ACT
            let (level, message) = parse_level_prefix(&line, LogLevel::Info);

            // ASSERT
            assert_eq!(level, LogLevel::Error, "level <{n}> should map to Error");
            assert_eq!(message, "critical message");
        }
    }

    #[test]
    fn parse_level_prefix_notice_maps_to_info() {
        // ARRANGE
        let line = "<5>normal but significant";

        // ACT
        let (level, message) = parse_level_prefix(line, LogLevel::Error);

        // ASSERT
        assert_eq!(level, LogLevel::Info);
        assert_eq!(message, "normal but significant");
    }

    #[test]
    fn parse_level_prefix_returns_default_for_unprefixed() {
        // ARRANGE
        let line = "just a normal message";

        // ACT
        let (level, message) = parse_level_prefix(line, LogLevel::Info);

        // ASSERT
        assert_eq!(level, LogLevel::Info);
        assert_eq!(message, "just a normal message");
    }

    #[test]
    fn parse_level_prefix_stderr_default_is_error() {
        // ARRANGE
        let line = "some stderr output";

        // ACT
        let (level, message) = parse_level_prefix(line, LogLevel::Error);

        // ASSERT
        assert_eq!(level, LogLevel::Error);
        assert_eq!(message, "some stderr output");
    }

    #[test]
    fn parse_level_prefix_invalid_number_uses_default() {
        // ARRANGE
        let line = "<99>some message";

        // ACT
        let (level, message) = parse_level_prefix(line, LogLevel::Info);

        // ASSERT
        assert_eq!(level, LogLevel::Info);
        assert_eq!(message, "some message");
    }

    #[test]
    fn parse_level_prefix_malformed_no_close_uses_default() {
        // ARRANGE
        let line = "<7 missing close bracket";

        // ACT
        let (level, message) = parse_level_prefix(line, LogLevel::Info);

        // ASSERT
        assert_eq!(level, LogLevel::Info);
        assert_eq!(message, "<7 missing close bracket");
    }

    #[tokio::test]
    async fn append_preserves_log_level() {
        // ARRANGE
        let (writer, reader, actor) = create();
        tokio::spawn(actor.run());

        // ACT
        writer.append("svc", LogStream::Stdout, LogLevel::Debug, "dbg".to_string());
        writer.append("svc", LogStream::Stdout, LogLevel::Warn, "wrn".to_string());
        writer.append("svc", LogStream::Stderr, LogLevel::Error, "err".to_string());

        let entries = reader.query(Some("svc".to_string()), 0).await;

        // ASSERT
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].level, LogLevel::Debug);
        assert_eq!(entries[1].level, LogLevel::Warn);
        assert_eq!(entries[2].level, LogLevel::Error);
    }

    #[test]
    fn log_level_ordering() {
        // ARRANGE & ACT & ASSERT
        assert!(LogLevel::Error < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Debug);
    }
}
