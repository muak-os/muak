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
pub fn capture(name: &'static str, stdout_fd: OwnedFd, stderr_fd: OwnedFd, logger: &LogWriter) {
    spawn_reader(name, stdout_fd, LogStream::Stdout, logger.clone());
    spawn_reader(name, stderr_fd, LogStream::Stderr, logger.clone());
}

fn spawn_reader(name: &'static str, fd: OwnedFd, stream: LogStream, logger: LogWriter) {
    tokio::spawn(async move {
        let async_fd = tokio::fs::File::from_std(std::fs::File::from(fd));
        let reader = BufReader::new(async_fd);
        let mut lines = reader.lines();

        while let Ok(Some(line)) = lines.next_line().await {
            logger.append(name, stream, line);
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
