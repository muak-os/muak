//! Kmsg - A small library for writing log messages to the kernel message buffer.
//!
//! This library provides direct access to `/dev/kmsg` for logging from early
//! boot processes and system daemons, with automatic fallback to stderr.

#![warn(missing_docs)]

extern crate alloc;

use alloc::borrow::Cow;
use std::io::Write as _;
use std::sync::OnceLock;

use rustix::fd::AsFd as _;
use rustix::fs::{Mode, OFlags, open};
use rustix::io::write;
use thiserror::Error;

/// Maximum atomic write size for `/dev/kmsg`.
const MAX_KMSG_SIZE: usize = 992;

const TRUNCATED_WARNING: &[u8] = b" [TRUNCATED]\n";
const KMSG_PATH_ENV: &str = "MUAK_KMSG_PATH";

#[repr(u8)]
#[derive(Debug, Clone, Copy)]
/// Kernel log severity levels.
pub enum Level {
    /// Error message (kernel log level 3).
    Error = 3,
    /// Warning message (kernel log level 4).
    Warn = 4,
    /// Informational message (kernel log level 6).
    Info = 6,
    /// Debug message (kernel log level 7).
    Debug = 7,
}

static DEFAULT_COMPONENT: OnceLock<String> = OnceLock::new();

impl From<Level> for u8 {
    fn from(level: Level) -> Self {
        match level {
            Level::Error => 3,
            Level::Warn => 4,
            Level::Info => 6,
            Level::Debug => 7,
        }
    }
}

fn kmsg_path() -> Cow<'static, str> {
    std::env::var(KMSG_PATH_ENV).map_or_else(|_err| Cow::Borrowed("/dev/kmsg"), Cow::Owned)
}

fn prepare_data_to_write(data: &[u8]) -> Cow<'_, [u8]> {
    if data.len() <= MAX_KMSG_SIZE {
        return Cow::Borrowed(data);
    }

    let available = MAX_KMSG_SIZE.saturating_sub(TRUNCATED_WARNING.len());
    let prefix = data.get(..available).unwrap_or(data);
    let mut truncated = Vec::with_capacity(MAX_KMSG_SIZE);
    truncated.extend_from_slice(prefix);
    truncated.extend_from_slice(TRUNCATED_WARNING);
    Cow::Owned(truncated)
}

fn write_to_kmsg_or_stderr(data: &[u8]) {
    let data_to_write = prepare_data_to_write(data);

    debug_assert!(
        data.len() <= MAX_KMSG_SIZE,
        "kmsg message exceeds MAX_KMSG_SIZE ({} bytes, got {} bytes). \
         Messages larger than {} bytes may be split or interleaved by the kernel. \
         Please make the message shorter!",
        MAX_KMSG_SIZE,
        data.len(),
        MAX_KMSG_SIZE
    );

    let written = open(
        kmsg_path().as_ref(),
        OFlags::WRONLY | OFlags::CLOEXEC | OFlags::APPEND,
        Mode::empty(),
    )
    .ok()
    .and_then(|file| write(file.as_fd(), data_to_write.as_ref()).ok())
    .is_some_and(|written| written == data_to_write.len());

    if !written {
        let mut stderr = std::io::stderr();
        let message = format_stderr_fallback_message(data);
        drop(stderr.write_all(message.as_bytes()));
    }
}

fn format_stderr_fallback_message(data: &[u8]) -> String {
    format!(
        "Failed to write to kmsg (falling back to stderr): {}",
        String::from_utf8_lossy(data)
    )
}

/// Initializes the default component used by logging macros and functions.
///
/// # Errors
///
/// Returns [`InitError::AlreadyInitialized`] when initialization has already happened.
pub fn init(component: &str) -> Result<(), InitError> {
    DEFAULT_COMPONENT
        .set(component.to_owned())
        .map_err(|_already_initialized| InitError::AlreadyInitialized)
}

/// Error type for logger initialization.
#[derive(Debug, Error)]
pub enum InitError {
    /// The kmsg logger has already been initialized.
    #[error("logger already initialized")]
    AlreadyInitialized,
}

fn format_log(level: Level, component: &str, message: &str) -> String {
    if component.is_empty() {
        format!("<{}> {}\n", u8::from(level), message)
    } else {
        format!("<{}>[{}] {}\n", u8::from(level), component, message)
    }
}

/// Writes a formatted log message to kmsg (or stderr as fallback).
pub fn write_log(level: Level, component: Option<&str>, message: &str) {
    let comp = component
        .or_else(|| DEFAULT_COMPONENT.get().map(String::as_str))
        .unwrap_or("");

    write_to_kmsg_or_stderr(format_log(level, comp, message).as_bytes());
}

/// Writes a plain message to kmsg with a trailing newline.
pub fn print(message: &str) {
    let formatted = format!("{message}\n");
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

/// Log a debug message when the `debug` feature is enabled; compiles away otherwise.
#[cfg(not(feature = "debug"))]
#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {};
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock as TestOnceLock};

    use super::*;

    static ENV_LOCK: TestOnceLock<Mutex<()>> = TestOnceLock::new();

    fn env_lock() -> &'static Mutex<()> {
        ENV_LOCK.get_or_init(|| Mutex::new(()))
    }

    struct KmsgPathGuard {
        previous: Option<OsString>,
    }

    impl KmsgPathGuard {
        fn set(path: &Path) -> Self {
            let previous = std::env::var_os(KMSG_PATH_ENV);
            // SAFETY: tests serialize env mutation with `ENV_LOCK`.
            unsafe {
                std::env::set_var(KMSG_PATH_ENV, path);
            }

            Self { previous }
        }
    }

    impl Drop for KmsgPathGuard {
        fn drop(&mut self) {
            match self.previous.as_deref() {
                // SAFETY: tests serialize env mutation with `ENV_LOCK`.
                Some(previous) => unsafe { std::env::set_var(KMSG_PATH_ENV, previous) },
                // SAFETY: tests serialize env mutation with `ENV_LOCK`.
                None => unsafe { std::env::remove_var(KMSG_PATH_ENV) },
            }
        }
    }

    fn temp_kmsg_file(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("kmsg-{name}-{}", std::process::id()));
        path
    }

    fn prepare_output_file(name: &str) -> PathBuf {
        let path = temp_kmsg_file(name);
        drop(fs::remove_file(&path));
        fs::write(&path, b"").expect("temporary kmsg file must be creatable");
        path
    }

    #[test]
    fn level_numeric_values() {
        // ARRANGE

        // ACT

        // ASSERT
        assert_eq!(u8::from(Level::Error), 3);
        assert_eq!(u8::from(Level::Warn), 4);
        assert_eq!(u8::from(Level::Info), 6);
        assert_eq!(u8::from(Level::Debug), 7);
    }

    #[test]
    fn level_debug_repr() {
        // ARRANGE

        // ACT

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
            assert_eq!(
                out.as_bytes().get(1),
                Some(&digit),
                "wrong priority for {level:?}"
            );
        }
    }

    #[test]
    fn format_log_message_preserved_verbatim() {
        // ARRANGE
        let msg = "Special: !@#$%^&*() and Unicode: \u{65E5}\u{672C}\u{8A9E}";

        // ACT
        let out = format_log(Level::Warn, "c", msg);

        // ASSERT
        assert!(
            out.contains(msg),
            "formatted output must preserve the original message"
        );
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
        // ARRANGE

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
    fn prepare_data_to_write_preserves_short_messages() {
        // ARRANGE
        let message = b"short message";

        // ACT
        let prepared = prepare_data_to_write(message);

        // ASSERT
        assert_eq!(
            prepared.as_ref(),
            message,
            "short messages must be written unchanged"
        );
        assert!(
            matches!(prepared, Cow::Borrowed(_)),
            "short messages should borrow the original slice"
        );
    }

    #[test]
    fn prepare_data_to_write_truncates_long_messages() {
        // ARRANGE
        let message = vec![b'x'; MAX_KMSG_SIZE + 32];

        // ACT
        let prepared = prepare_data_to_write(&message);

        // ASSERT
        assert_eq!(
            prepared.len(),
            MAX_KMSG_SIZE,
            "truncated messages must respect MAX_KMSG_SIZE"
        );
        assert!(
            prepared.ends_with(TRUNCATED_WARNING),
            "truncated messages must include the truncation marker"
        );
        assert!(
            matches!(prepared, Cow::Owned(_)),
            "truncation should allocate a new buffer"
        );
    }

    #[test]
    fn format_stderr_fallback_message_includes_original_payload() {
        // ARRANGE
        let payload = b"<6>[svc] hello\n";

        // ACT
        let message = format_stderr_fallback_message(payload);

        // ASSERT
        assert!(
            message.starts_with("Failed to write to kmsg"),
            "fallback messages must explain the stderr fallback"
        );
        assert!(
            message.contains("[svc] hello"),
            "fallback messages must include the original payload"
        );
    }

    #[test]
    fn init_fails_on_second_call() {
        // ARRANGE
        let first_result = init("svc");

        // ACT
        let second_result = init("other");

        // ASSERT
        assert!(
            matches!(first_result, Ok(()))
                || matches!(first_result, Err(InitError::AlreadyInitialized)),
            "the first initialization must either succeed or observe prior initialization"
        );
        assert!(
            matches!(second_result, Err(InitError::AlreadyInitialized)),
            "the second initialization must return AlreadyInitialized"
        );
    }

    #[test]
    fn init_error_display_and_debug() {
        // ARRANGE
        let err = InitError::AlreadyInitialized;

        // ASSERT
        assert!(
            format!("{err:?}").contains("AlreadyInitialized"),
            "debug output must include the variant name"
        );
        assert!(
            !err.to_string().is_empty(),
            "display output must not be empty"
        );
    }

    #[test]
    fn public_api_writes_formatted_logs_to_configured_path() {
        // ARRANGE
        let _guard = env_lock().lock().expect("env lock must not be poisoned");
        let path = prepare_output_file("formatted");
        let _path_guard = KmsgPathGuard::set(&path);

        // ACT
        write_log(Level::Info, Some("svc"), "hello");
        write_log(Level::Error, None, "oops");

        // ASSERT
        let contents = fs::read_to_string(&path).expect("configured kmsg output must be readable");
        assert_eq!(
            contents, "<6>[svc] hello\n<3> oops\n",
            "configured kmsg output must preserve the formatted wire format"
        );
        fs::remove_file(&path).expect("temporary kmsg file must be removable");
    }

    #[test]
    fn print_writes_plain_message_with_newline() {
        // ARRANGE
        let _guard = env_lock().lock().expect("env lock must not be poisoned");
        let path = prepare_output_file("print");
        let _path_guard = KmsgPathGuard::set(&path);

        // ACT
        print("plain");

        // ASSERT
        let contents = fs::read_to_string(&path).expect("configured kmsg output must be readable");
        assert_eq!(
            contents, "plain\n",
            "print must append exactly one trailing newline"
        );
        fs::remove_file(&path).expect("temporary kmsg file must be removable");
    }

    #[test]
    fn macros_write_expected_levels_and_components() {
        // ARRANGE
        let _guard = env_lock().lock().expect("env lock must not be poisoned");
        let path = prepare_output_file("macros");
        let _path_guard = KmsgPathGuard::set(&path);

        // ACT
        error!("failed {}", 1);
        warn!(@ "net", "warn {}", 2);
        info!("ready {}", 3);
        debug!("trace {}", 4);

        // ASSERT
        let contents = fs::read_to_string(&path).expect("configured kmsg output must be readable");
        assert!(
            contents.contains("<3> failed 1\n"),
            "error! must emit priority 3 without brackets when no component is provided"
        );
        assert!(
            contents.contains("<4>[net] warn 2\n"),
            "warn! must emit priority 4 with an explicit component"
        );
        assert!(
            contents.contains("<6> ready 3\n"),
            "info! must emit priority 6 without brackets when no component is provided"
        );
        assert!(
            !contents.contains("trace 4") || contents.contains("<7> trace 4\n"),
            "debug! must either compile away or emit a priority-7 line"
        );
        fs::remove_file(&path).expect("temporary kmsg file must be removable");
    }

    #[test]
    fn write_debug_level_to_configured_path() {
        // ARRANGE
        let _guard = env_lock().lock().expect("env lock must not be poisoned");
        let path = prepare_output_file("debug-level");
        let _path_guard = KmsgPathGuard::set(&path);

        // ACT
        write_log(Level::Debug, Some("dbg"), "trace");

        // ASSERT
        let contents = fs::read_to_string(&path).expect("configured kmsg output must be readable");
        assert_eq!(
            contents, "<7>[dbg] trace\n",
            "debug-level logs must encode priority 7 with the supplied component"
        );
        fs::remove_file(&path).expect("temporary kmsg file must be removable");
    }

    #[test]
    fn falls_back_cleanly_when_open_fails() {
        // ARRANGE
        let _guard = env_lock().lock().expect("env lock must not be poisoned");
        let mut missing_path = std::env::temp_dir();
        missing_path.push(format!("missing-dir-{}/kmsg", std::process::id()));
        let _path_guard = KmsgPathGuard::set(&missing_path);

        // ACT
        let result = std::panic::catch_unwind(|| {
            write_log(Level::Info, Some("svc"), "fallback");
            print("plain");
        });

        // ASSERT
        assert!(
            result.is_ok(),
            "logging must not panic when the kmsg device cannot be opened"
        );
    }

    #[test]
    fn falls_back_cleanly_with_default_kmsg_path() {
        // ARRANGE
        let _guard = env_lock().lock().expect("env lock must not be poisoned");
        let previous = std::env::var_os(KMSG_PATH_ENV);
        // SAFETY: tests serialize env mutation with `ENV_LOCK`.
        unsafe {
            std::env::remove_var(KMSG_PATH_ENV);
        }

        // ACT
        let result = std::panic::catch_unwind(|| {
            write_log(Level::Info, Some("svc"), "default-path");
        });

        if let Some(previous) = previous {
            // SAFETY: tests serialize env mutation with `ENV_LOCK`.
            unsafe {
                std::env::set_var(KMSG_PATH_ENV, previous);
            }
        }

        // ASSERT
        assert!(
            result.is_ok(),
            "logging must not panic when using the default kmsg path"
        );
    }

    #[test]
    fn uses_default_component_after_init() {
        // ARRANGE
        let _guard = env_lock().lock().expect("env lock must not be poisoned");
        let path = prepare_output_file("default-component");
        let _path_guard = KmsgPathGuard::set(&path);
        let init_result = init("svc");

        // ACT
        write_log(Level::Warn, None, "degraded");

        // ASSERT
        assert!(
            matches!(init_result, Ok(()))
                || matches!(init_result, Err(InitError::AlreadyInitialized)),
            "the logger must initialize or already be initialized"
        );
        let contents = fs::read_to_string(&path).expect("configured kmsg output must be readable");
        assert_eq!(
            contents, "<4>[svc] degraded\n",
            "write_log must use the initialized default component when none is provided"
        );
        fs::remove_file(&path).expect("temporary kmsg file must be removable");
    }

    #[test]
    #[cfg(debug_assertions)]
    fn rejects_oversized_messages_in_debug() {
        // ARRANGE
        let message = "x".repeat(993);

        // ACT
        let result = std::panic::catch_unwind(|| {
            write_log(Level::Info, Some("svc"), &message);
        });

        // ASSERT
        assert!(
            result.is_err(),
            "oversized messages must panic in debug builds"
        );
    }
}
