//! Kmsg - A small library for writing log messages to the kernel message buffer
//!
//! This library provides direct access to `/dev/kmsg` for logging from early
//! boot processes and system daemons, with automatic fallback to stderr.

use std::io::Write;
use std::sync::OnceLock;

use rustix::fd::AsFd;
use rustix::fs::{Mode, OFlags, open};
use rustix::io::write;
use thiserror::Error;

/// Maximum atomic write size for /dev/kmsg guaranteed by the Linux kernel.
/// This is LOG_LINE_MAX (1024) - PREFIX_MAX (32) = 992 bytes.
/// Writes larger than this may be split or interleaved by the kernel.
/// See kernel Documentation: Documentation/ABI/testing/dev-kmsg
const MAX_KMSG_SIZE: usize = 992;

#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum Level {
    Error = 3,
    Warn = 4,
    Info = 6,
    Debug = 7,
}

static DEFAULT_COMPONENT: OnceLock<String> = OnceLock::new();

fn write_to_kmsg_or_stderr(data: &[u8]) {
    #[cfg(debug_assertions)]
    {
        if data.len() > MAX_KMSG_SIZE {
            panic!(
                "kmsg message exceeds MAX_KMSG_SIZE ({} bytes, got {} bytes). \
                 Messages larger than {} bytes may be split or interleaved by the kernel. \
                 Please make the message shorter!",
                MAX_KMSG_SIZE,
                data.len(),
                MAX_KMSG_SIZE
            );
        }
    }

    let data_to_write: &[u8] = if data.len() > MAX_KMSG_SIZE {
        const WARNING: &[u8] = b" [TRUNCATED]\n";
        let available = MAX_KMSG_SIZE.saturating_sub(WARNING.len());

        let mut buf = Vec::with_capacity(MAX_KMSG_SIZE);
        buf.extend_from_slice(&data[..available]);
        buf.extend_from_slice(WARNING);

        Box::leak(buf.into_boxed_slice())
    } else {
        data
    };

    let mut written = false;
    if let Ok(file) = open("/dev/kmsg", OFlags::WRONLY | OFlags::CLOEXEC, Mode::empty()) {
        written = write(file.as_fd(), data_to_write)
            .map(|n| n == data_to_write.len())
            .unwrap_or(false);
    }

    if !written {
        let _ = std::io::stderr().write_all(
            format!(
                "Failed to write to kmsg (falling back to stderr): {}",
                String::from_utf8_lossy(data)
            )
            .as_bytes(),
        );
    }
}

/// Initialize the logger with a default component name.
///
/// This sets the default component name used when logging without
/// an explicit component. The logger writes directly to `/dev/kmsg`
/// on each log call, making it safe to use across fork boundaries.
///
/// # Arguments
///
/// * `component` - Default component name to use in log messages (e.g., "init", "granola")
///
/// # Errors
///
/// Returns an error if the logger has already been initialized.
///
/// # Example
///
/// ```rust,ignore
/// kmsg::init("myapp")?;
/// ```
pub fn init(component: &str) -> Result<(), InitError> {
    DEFAULT_COMPONENT
        .set(component.to_string())
        .map_err(|_| InitError::AlreadyInitialized)
}

#[derive(Debug, Error)]
pub enum InitError {
    #[error("logger already initialized")]
    AlreadyInitialized,
}

/// Formats a log message into the kernel printk wire format.
///
/// Returns `<LEVEL> MESSAGE\n` when `component` is empty, or
/// `<LEVEL>[COMPONENT] MESSAGE\n` otherwise.
pub(crate) fn format_log(level: Level, component: &str, message: &str) -> String {
    if component.is_empty() {
        format!("<{}> {}\n", level as u8, message)
    } else {
        format!("<{}>[{}] {}\n", level as u8, component, message)
    }
}

/// Writes a log message with the specified level and optional component.
pub fn write_log(level: Level, component: Option<&str>, message: &str) {
    let comp = component
        .or_else(|| DEFAULT_COMPONENT.get().map(|s| s.as_str()))
        .unwrap_or("");

    write_to_kmsg_or_stderr(format_log(level, comp, message).as_bytes());
}

/// Prints a plain message to kmsg without priority prefix.
pub fn print(message: &str) {
    let formatted = format!("{}\n", message);
    write_to_kmsg_or_stderr(formatted.as_bytes());
}

/// Log an error message (priority 3).
///
/// # Examples
///
/// ```rust,ignore
/// // Using default component
/// kmsg::error!("Failed to open file: {}", err);
///
/// // Using dynamic component (@ prefix)
/// kmsg::error!(@ "network", "Connection failed: {}", err);
/// ```
#[macro_export]
macro_rules! error {
    (@ $component:expr, $($arg:tt)*) => {
        $crate::write_log($crate::Level::Error, Some($component), &format!($($arg)*))
    };
    ($($arg:tt)*) => {
        $crate::write_log($crate::Level::Error, None, &format!($($arg)*))
    };
}

/// Log a warning message (priority 4).
///
/// # Examples
///
/// ```rust,ignore
/// // Using default component
/// kmsg::warn!("Configuration file not found, using defaults");
///
/// // Using dynamic component (@ prefix)
/// kmsg::warn!(@ "config", "Missing key: {}", key);
/// ```
#[macro_export]
macro_rules! warn {
    (@ $component:expr, $($arg:tt)*) => {
        $crate::write_log($crate::Level::Warn, Some($component), &format!($($arg)*))
    };
    ($($arg:tt)*) => {
        $crate::write_log($crate::Level::Warn, None, &format!($($arg)*))
    };
}

/// Log an informational message (priority 6).
///
/// # Examples
///
/// ```rust,ignore
/// // Using default component
/// kmsg::info!("Server started on port {}", port);
///
/// // Using dynamic component (@ prefix)
/// kmsg::info!(@ "vm-manager", "VM {} is now running", vm_name);
/// ```
#[macro_export]
macro_rules! info {
    (@ $component:expr, $($arg:tt)*) => {
        $crate::write_log($crate::Level::Info, Some($component), &format!($($arg)*))
    };
    ($($arg:tt)*) => {
        $crate::write_log($crate::Level::Info, None, &format!($($arg)*))
    };
}

/// Log a debug message (priority 7).
///
/// This macro is only enabled when the `debug` feature is active.
/// When the feature is disabled, calls to `debug!` compile to nothing.
///
/// # Examples
///
/// ```rust,ignore
/// // Using default component
/// kmsg::debug!("Entering function with args: {:?}", args);
///
/// // Using dynamic component (@ prefix)
/// kmsg::debug!(@ "parser", "Token: {:?}", token);
/// ```
#[cfg(feature = "debug")]
#[macro_export]
macro_rules! debug {
    (@ $component:expr, $($arg:tt)*) => {
        $crate::write_log($crate::Level::Debug, Some($component), &format!($($arg)*))
    };
    ($($arg:tt)*) => {
        $crate::write_log($crate::Level::Debug, None, &format!($($arg)*))
    };
}

#[cfg(not(feature = "debug"))]
#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_numeric_values() {
        // ASSERT
        assert_eq!(Level::Error as u8, 3);
        assert_eq!(Level::Warn as u8, 4);
        assert_eq!(Level::Info as u8, 6);
        assert_eq!(Level::Debug as u8, 7);
    }

    #[test]
    fn level_debug_repr() {
        // ASSERT
        assert_eq!(format!("{:?}", Level::Error), "Error");
        assert_eq!(format!("{:?}", Level::Warn), "Warn");
        assert_eq!(format!("{:?}", Level::Info), "Info");
        assert_eq!(format!("{:?}", Level::Debug), "Debug");
    }

    #[test]
    fn format_log_with_component_encodes_level_and_tag() {
        // ACT
        let out = format_log(Level::Info, "myapp", "hello");

        // ASSERT
        assert_eq!(out, "<6>[myapp] hello\n");
    }

    #[test]
    fn format_log_without_component_omits_brackets() {
        // ACT
        let out = format_log(Level::Error, "", "oops");

        // ASSERT
        assert_eq!(out, "<3> oops\n");
    }

    #[test]
    fn format_log_all_levels_produce_correct_priority_byte() {
        // ARRANGE
        let cases = [
            (Level::Error, b'3'),
            (Level::Warn, b'4'),
            (Level::Info, b'6'),
            (Level::Debug, b'7'),
        ];

        // ACT & ASSERT
        for (level, digit) in cases {
            let out = format_log(level, "t", "m");
            assert_eq!(out.as_bytes()[1], digit, "wrong priority for {level:?}");
        }
    }

    #[test]
    fn format_log_message_preserved_verbatim() {
        // ARRANGE
        let msg = "Special: !@#$%^&*() and Unicode: \u{65E5}\u{672C}\u{8A9E}";

        // ACT
        let out = format_log(Level::Warn, "c", msg);

        // ASSERT
        assert!(out.contains(msg));
    }

    #[test]
    fn format_log_component_with_punctuation() {
        // ACT
        let out = format_log(Level::Info, "my-component.v1", "test");

        // ASSERT
        assert_eq!(out, "<6>[my-component.v1] test\n");
    }

    #[test]
    fn format_log_empty_message() {
        // ACT
        let out = format_log(Level::Info, "svc", "");

        // ASSERT
        assert_eq!(out, "<6>[svc] \n");
    }

    #[test]
    fn format_log_always_ends_with_newline() {
        // ACT & ASSERT
        for comp in ["", "c"] {
            let out = format_log(Level::Info, comp, "msg");
            assert!(out.ends_with('\n'), "missing newline for comp={comp:?}");
        }
    }

    #[test]
    fn format_log_within_max_kmsg_size() {
        // ARRANGE
        let prefix_overhead = "<6>[test] \n".len();
        let safe_size = MAX_KMSG_SIZE - prefix_overhead - 1;
        let msg = "x".repeat(safe_size);

        // ACT
        let out = format_log(Level::Info, "test", &msg);

        // ASSERT
        assert!(
            out.len() <= MAX_KMSG_SIZE,
            "formatted output exceeds MAX_KMSG_SIZE"
        );
    }

    #[test]
    fn init_fails_on_second_call() {
        // ARRANGE
        let already_initialized = DEFAULT_COMPONENT.get().is_some();

        if already_initialized {
            // ACT
            let result = init("second-init");

            // ASSERT
            assert!(result.is_err());
            assert!(matches!(result.unwrap_err(), InitError::AlreadyInitialized));
        } else {
            // ACT
            let first = init("first-init");
            assert!(first.is_ok());

            let second = init("second-init");

            // ASSERT
            assert!(second.is_err());
            assert!(matches!(second.unwrap_err(), InitError::AlreadyInitialized));
        }
    }

    #[test]
    fn init_error_display_and_debug() {
        // ARRANGE
        let err = InitError::AlreadyInitialized;

        // ASSERT
        assert!(format!("{:?}", err).contains("AlreadyInitialized"));
        assert!(!err.to_string().is_empty());
    }

    #[test]
    #[cfg(debug_assertions)]
    fn message_truncation_panics_in_debug() {
        // ARRANGE
        let long_message = "x".repeat(MAX_KMSG_SIZE + 100);

        // ACT
        let result = std::panic::catch_unwind(|| {
            write_log(Level::Info, Some("test"), &long_message);
        });

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    #[cfg(not(debug_assertions))]
    fn message_truncation_in_release_appends_marker() {
        // ARRANGE
        let long_message = "x".repeat(MAX_KMSG_SIZE + 100);

        // ACT
        let formatted = format_log(Level::Info, "test", &long_message);

        // ASSERT
        assert!(formatted.len() > MAX_KMSG_SIZE);
        write_log(Level::Info, Some("test"), &long_message);
    }
}
