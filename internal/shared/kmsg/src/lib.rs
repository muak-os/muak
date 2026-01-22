use std::fs::OpenOptions;
use std::io::{self, Write};
use std::sync::OnceLock;
use thiserror::Error;

#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum Level {
    Error = 3,
    Warn = 4,
    Info = 6,
    Debug = 7,
}

static DEFAULT_COMPONENT: OnceLock<String> = OnceLock::new();

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

pub fn write_log(level: Level, component: Option<&str>, message: &str) {
    let comp = component
        .map(|s| s.to_string())
        .or_else(|| DEFAULT_COMPONENT.get().cloned())
        .unwrap_or_default();

    let formatted = if comp.is_empty() {
        format!("<{}> {}\n", level as u8, message)
    } else {
        format!("<{}>[{}] {}\n", level as u8, comp, message)
    };

    let written = if let Ok(mut kmsg) = OpenOptions::new().write(true).open("/dev/kmsg") {
        kmsg.write_all(formatted.as_bytes()).is_ok()
    } else {
        false
    };

    if !written {
        let _ = io::stderr().write_all(formatted.as_bytes());
    }
}

pub fn print(message: &str) {
    let formatted = format!("{}\n", message);

    let written = if let Ok(mut kmsg) = OpenOptions::new().write(true).open("/dev/kmsg") {
        kmsg.write_all(formatted.as_bytes()).is_ok()
    } else {
        false
    };

    if !written {
        let _ = io::stderr().write_all(formatted.as_bytes());
    }
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
