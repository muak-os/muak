//! Feeds process output and kernel messages into the log actor.

use std::os::fd::OwnedFd;

use tokio::io::{AsyncBufReadExt as _, BufReader};

use super::{LogLevel, LogStream, LogWriter};

/// Captures logs from the given stdout and stderr file descriptors, forwards them to the logger.
pub fn capture(name: &str, stdout_fd: OwnedFd, stderr_fd: OwnedFd, logger: &LogWriter) {
    spawn_reader(
        name.to_owned(),
        stdout_fd,
        LogStream::Stdout,
        logger.clone(),
    );
    spawn_reader(
        name.to_owned(),
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
    let Some((number, message)) = rest.split_once('>') else {
        return (default, line);
    };
    let Ok(number) = number.parse::<u8>() else {
        return (default, line);
    };
    let level = LogLevel::try_from(number).unwrap_or(default);

    (level, message)
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
            logger.append(&name, stream, level, message.to_owned());
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
            Ok(opened) => opened,
            Err(e) => {
                kmsg::warn!("Failed to open /dev/kmsg for log capture: {e}");
                return;
            }
        };

        let reader = BufReader::new(file);
        let mut lines = reader.lines();

        while let Ok(Some(line)) = lines.next_line().await {
            let message = match line.split_once(';') {
                Some((_, message)) => message,
                None => line.as_str(),
            };
            logger.append(
                "kernel",
                LogStream::Stdout,
                LogLevel::Info,
                message.to_owned(),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

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
        for number in 0_u8..=2 {
            let line = format!("<{number}>critical message");

            // ACT
            let (level, message) = parse_level_prefix(&line, LogLevel::Info);

            // ASSERT
            assert_eq!(
                level,
                LogLevel::Error,
                "level <{number}> should map to Error"
            );
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
}
