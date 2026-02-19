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

/// Writes a log message with the specified level and optional component.
pub fn write_log(level: Level, component: Option<&str>, message: &str) {
    let comp = component
        .or_else(|| DEFAULT_COMPONENT.get().map(|s| s.as_str()))
        .unwrap_or("");

    let formatted = if comp.is_empty() {
        format!("<{}> {}\n", level as u8, message)
    } else {
        format!("<{}>[{}] {}\n", level as u8, comp, message)
    };

    write_to_kmsg_or_stderr(formatted.as_bytes());
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
    fn test_level_debug_trait() {
        let levels = vec![Level::Error, Level::Warn, Level::Info, Level::Debug];
        for level in levels {
            let debug_str = format!("{:?}", level);
            assert!(!debug_str.is_empty());
        }
    }

    #[test]
    fn test_init_fails_on_second_call() {
        if DEFAULT_COMPONENT.get().is_some() {
            let result = init("second-init");
            assert!(result.is_err());
            assert!(matches!(result.unwrap_err(), InitError::AlreadyInitialized));
        } else {
            let first = init("first-init");
            assert!(first.is_ok());
            let second = init("second-init");
            assert!(second.is_err());
        }
    }

    #[test]
    fn test_write_log_with_component() {
        let result = std::panic::catch_unwind(|| {
            write_log(Level::Info, Some("my-component"), "test message");
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_write_log_without_component() {
        let result = std::panic::catch_unwind(|| {
            write_log(Level::Error, None, "error message");
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_write_log_all_levels() {
        for level in [Level::Error, Level::Warn, Level::Info, Level::Debug] {
            let result = std::panic::catch_unwind(|| {
                write_log(level, Some("test"), "message");
            });
            assert!(result.is_ok(), "Failed for level {:?}", level);
        }
    }

    #[test]
    fn test_print_function() {
        let result = std::panic::catch_unwind(|| {
            print("simple print message");
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_print_with_newlines() {
        let result = std::panic::catch_unwind(|| {
            print("line1\nline2");
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_print_empty_string() {
        let result = std::panic::catch_unwind(|| {
            print("");
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_error_macro_without_component() {
        let result = std::panic::catch_unwind(|| {
            error!("Test error message");
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_error_macro_with_component() {
        let result = std::panic::catch_unwind(|| {
            error!(@ "network", "Connection failed");
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_error_macro_with_format_args() {
        let result = std::panic::catch_unwind(|| {
            error!("Error code: {}", 404);
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_warn_macro_without_component() {
        let result = std::panic::catch_unwind(|| {
            warn!("Test warning message");
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_warn_macro_with_component() {
        let result = std::panic::catch_unwind(|| {
            warn!(@ "config", "Missing key");
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_warn_macro_with_format_args() {
        let result = std::panic::catch_unwind(|| {
            warn!("Warning: {} warnings found", 5);
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_info_macro_without_component() {
        let result = std::panic::catch_unwind(|| {
            info!("Test info message");
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_info_macro_with_component() {
        let result = std::panic::catch_unwind(|| {
            info!(@ "server", "Server started");
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_info_macro_with_format_args() {
        let result = std::panic::catch_unwind(|| {
            info!("Port: {}", 8080);
        });
        assert!(result.is_ok());
    }

    #[test]
    #[cfg(feature = "debug")]
    fn test_debug_macro_without_component() {
        let result = std::panic::catch_unwind(|| {
            debug!("Test debug message");
        });
        assert!(result.is_ok());
    }

    #[test]
    #[cfg(feature = "debug")]
    fn test_debug_macro_with_component() {
        let result = std::panic::catch_unwind(|| {
            debug!(@ "parser", "Parsing token");
        });
        assert!(result.is_ok());
    }

    #[test]
    #[cfg(feature = "debug")]
    fn test_debug_macro_with_format_args() {
        let result = std::panic::catch_unwind(|| {
            debug!("Debug value: {:?}", vec![1, 2, 3]);
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_long_message_no_panic() {
        let prefix_overhead = "<6>[test] \n".len();
        let safe_size = MAX_KMSG_SIZE - prefix_overhead - 1;
        let long_message = "x".repeat(safe_size);
        let result = std::panic::catch_unwind(|| {
            write_log(Level::Info, Some("test"), &long_message);
        });
        assert!(result.is_ok());
    }

    #[test]
    #[cfg(not(debug_assertions))]
    fn test_message_truncation_in_release() {
        let long_message = "x".repeat(MAX_KMSG_SIZE + 100);
        write_log(Level::Info, Some("test"), &long_message);
    }

    #[test]
    #[cfg(debug_assertions)]
    fn test_message_truncation_panics_in_debug() {
        let long_message = "x".repeat(MAX_KMSG_SIZE + 100);
        let result = std::panic::catch_unwind(|| {
            write_log(Level::Info, Some("test"), &long_message);
        });
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_message() {
        let result = std::panic::catch_unwind(|| {
            write_log(Level::Info, Some("test"), "");
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_message_with_special_chars() {
        let result = std::panic::catch_unwind(|| {
            write_log(Level::Info, Some("test"), "Special: !@#$%^&*()");
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_message_with_unicode() {
        let result = std::panic::catch_unwind(|| {
            write_log(Level::Info, Some("test"), "Unicode: 日本語 🎉");
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_component_with_special_chars() {
        let result = std::panic::catch_unwind(|| {
            write_log(Level::Info, Some("my-component.v1"), "test");
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_nested_formatting() {
        let result = std::panic::catch_unwind(|| {
            info!("Values: {}, {:?}, {}", 42, "test", true);
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_init_error_debug() {
        let err = InitError::AlreadyInitialized;
        let debug_str = format!("{:?}", err);
        assert!(debug_str.contains("AlreadyInitialized"));
    }
}
