use std::fs::File;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::OnceLock;
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
static KMSG_FILE: OnceLock<Option<File>> = OnceLock::new();

fn get_kmsg() -> &'static Option<File> {
    KMSG_FILE.get_or_init(|| OpenOptions::new().write(true).open("/dev/kmsg").ok())
}

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

        // Note: This creates a temporary Vec, but only in the exceptional case
        // where a message is too large (which should be rare).
        let mut buf = Vec::with_capacity(MAX_KMSG_SIZE);
        buf.extend_from_slice(&data[..available]);
        buf.extend_from_slice(WARNING);

        Box::leak(buf.into_boxed_slice())
    } else {
        data
    };

    let mut written = false;
    if let Some(kmsg) = get_kmsg().as_ref() {
        written = (&*kmsg).write_all(data_to_write).is_ok();

        if !written {
            written = (&*kmsg).write_all(data_to_write).is_ok();
        }
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

    write_to_kmsg_or_stderr(formatted.as_bytes());
}

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
