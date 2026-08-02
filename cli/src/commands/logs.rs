use anyhow::Result;
use tonic::transport::Channel;

use crate::client::log_service::{
    FollowLogsRequest, GetLogsRequest, GetLogsResponse, Level, LogEntry,
    log_service_client::LogServiceClient,
};
use crate::ui;

/// Parses a log level name into its proto integer value.
pub fn parse_level(level: &str) -> Result<i32, String> {
    match level.to_lowercase().as_str() {
        "error" => Ok(level_value(Level::Error)),
        "warn" => Ok(level_value(Level::Warn)),
        "info" => Ok(level_value(Level::Info)),
        "debug" => Ok(level_value(Level::Debug)),
        _ => Err(format!(
            "invalid level '{level}' (expected: error, warn, info, debug)"
        )),
    }
}

/// Converts a proto log level enum to its integer value.
///
/// The discriminants mirror the values defined in the `log.proto` enum.
fn level_value(level: Level) -> i32 {
    match level {
        Level::Error => 0,
        Level::Warn => 1,
        Level::Info => 2,
        Level::Debug => 3,
    }
}

/// Fetches and displays service logs.
pub async fn handle(
    client: &mut LogServiceClient<Channel>,
    service: Option<String>,
    tail: Option<u32>,
    follow: bool,
    level: Option<i32>,
) -> Result<()> {
    if follow {
        handle_follow(client, service, level).await
    } else {
        handle_get(client, service, tail.unwrap_or(0), level).await
    }
}

/// Fetches a batch of recent logs.
async fn handle_get(
    client: &mut LogServiceClient<Channel>,
    service: Option<String>,
    tail: u32,
    level: Option<i32>,
) -> Result<()> {
    let request = tonic::Request::new(GetLogsRequest {
        service: service.unwrap_or_default(),
        tail,
    });

    let GetLogsResponse { entries } = client.get_logs(request).await?.into_inner();

    for entry in &entries {
        if should_display(entry, level) {
            print_entry(entry);
        }
    }

    Ok(())
}

/// Follows live log output (like `tail -f` / `dmesg -w`).
async fn handle_follow(
    client: &mut LogServiceClient<Channel>,
    service: Option<String>,
    level: Option<i32>,
) -> Result<()> {
    let request = tonic::Request::new(FollowLogsRequest {
        service: service.unwrap_or_default(),
    });

    let mut stream = client.follow_logs(request).await?.into_inner();

    while let Some(entry) = stream.message().await? {
        if should_display(&entry, level) {
            print_entry(&entry);
        }
    }

    Ok(())
}

/// Returns true if the entry's level is at or below the threshold.
fn should_display(entry: &LogEntry, max_level: Option<i32>) -> bool {
    match max_level {
        Some(threshold) => entry.level <= threshold,
        None => entry.level <= level_value(Level::Info),
    }
}

/// Formats and prints a single log entry with level-based coloring.
fn print_entry(entry: &LogEntry) {
    let line = format!("[{}] {}", entry.service, entry.message);

    match Level::try_from(entry.level) {
        Ok(Level::Error) => println!("{}", ui::style::error_text(&line)),
        Ok(Level::Warn) => println!("{}", ui::style::warn(&line)),
        Ok(Level::Debug) => println!("{}", ui::style::muted(&line)),
        _ => println!("{line}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_level_accepts_all_valid_names() {
        // ARRANGE & ACT & ASSERT
        assert_eq!(parse_level("error").unwrap(), level_value(Level::Error));
        assert_eq!(parse_level("warn").unwrap(), level_value(Level::Warn));
        assert_eq!(parse_level("info").unwrap(), level_value(Level::Info));
        assert_eq!(parse_level("debug").unwrap(), level_value(Level::Debug));
    }

    #[test]
    fn parse_level_is_case_insensitive() {
        // ARRANGE & ACT & ASSERT
        assert_eq!(parse_level("DEBUG").unwrap(), level_value(Level::Debug));
        assert_eq!(parse_level("Error").unwrap(), level_value(Level::Error));
    }

    #[test]
    fn parse_level_rejects_invalid_name() {
        // ARRANGE & ACT
        let result = parse_level("trace");

        // ASSERT
        result.unwrap_err();
    }

    fn make_entry(level: Level) -> LogEntry {
        LogEntry {
            timestamp: 0,
            service: "svc".to_owned(),
            stream: 0,
            level: level_value(level),
            message: "msg".to_owned(),
        }
    }

    #[test]
    fn should_display_defaults_to_info_threshold() {
        // ARRANGE
        let error = make_entry(Level::Error);
        let warn = make_entry(Level::Warn);
        let info = make_entry(Level::Info);
        let debug = make_entry(Level::Debug);

        // ACT & ASSERT
        assert!(should_display(&error, None));
        assert!(should_display(&warn, None));
        assert!(should_display(&info, None));
        assert!(!should_display(&debug, None));
    }

    #[test]
    fn should_display_with_debug_threshold_shows_all() {
        // ARRANGE
        let threshold = Some(level_value(Level::Debug));

        // ACT & ASSERT
        assert!(should_display(&make_entry(Level::Error), threshold));
        assert!(should_display(&make_entry(Level::Warn), threshold));
        assert!(should_display(&make_entry(Level::Info), threshold));
        assert!(should_display(&make_entry(Level::Debug), threshold));
    }

    #[test]
    fn should_display_with_error_threshold_shows_only_errors() {
        // ARRANGE
        let threshold = Some(level_value(Level::Error));

        // ACT & ASSERT
        assert!(should_display(&make_entry(Level::Error), threshold));
        assert!(!should_display(&make_entry(Level::Warn), threshold));
        assert!(!should_display(&make_entry(Level::Info), threshold));
        assert!(!should_display(&make_entry(Level::Debug), threshold));
    }

    #[test]
    fn should_display_with_warn_threshold_shows_error_and_warn() {
        // ARRANGE
        let threshold = Some(level_value(Level::Warn));

        // ACT & ASSERT
        assert!(should_display(&make_entry(Level::Error), threshold));
        assert!(should_display(&make_entry(Level::Warn), threshold));
        assert!(!should_display(&make_entry(Level::Info), threshold));
        assert!(!should_display(&make_entry(Level::Debug), threshold));
    }
}
