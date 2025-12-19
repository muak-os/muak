use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::sync::{Mutex, OnceLock};

#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum Level {
    Error = 3,
    Warn = 4,
    Info = 6,
    Debug = 7,
}

struct Logger {
    kmsg: Option<File>,
    component: String,
}

static LOGGER: OnceLock<Mutex<Logger>> = OnceLock::new();

/// Initialize the logger with a default component name.
///
/// Opens `/dev/kmsg` for writing. If `/dev/kmsg` is unavailable,
/// the logger will fall back to stderr.
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
    let kmsg = OpenOptions::new().write(true).open("/dev/kmsg").ok();
    let logger = Logger {
        kmsg,
        component: component.to_string(),
    };

    LOGGER
        .set(Mutex::new(logger))
        .map_err(|_| InitError::AlreadyInitialized)
}

#[derive(Debug)]
pub enum InitError {
    AlreadyInitialized,
}

impl std::fmt::Display for InitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyInitialized => write!(f, "logger already initialized"),
        }
    }
}

impl std::error::Error for InitError {}

pub fn write_log(level: Level, component: Option<&str>, message: &str) {
    let formatted = if let Some(comp) = component {
        format!("<{}>[{}] {}\n", level as u8, comp, message)
    } else if let Some(logger) = LOGGER.get() {
        if let Ok(guard) = logger.lock() {
            format!("<{}>[{}] {}\n", level as u8, guard.component, message)
        } else {
            format!("<{}> {}\n", level as u8, message)
        }
    } else {
        format!("<{}> {}\n", level as u8, message)
    };

    let written = if let Some(logger) = LOGGER.get() {
        if let Ok(mut guard) = logger.lock() {
            if let Some(ref mut kmsg) = guard.kmsg {
                kmsg.write_all(formatted.as_bytes()).is_ok() && kmsg.flush().is_ok()
            } else {
                false
            }
        } else {
            false
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_init_twice_fails() {
        let result1 = init("test");
        assert!(result1.is_ok());
        let result2 = init("test2");
        assert!(matches!(result2, Err(InitError::AlreadyInitialized)));
    }
}
