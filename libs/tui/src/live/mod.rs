//! Live single-line widgets.

mod renderer;
mod spinner;
mod steps;

use crate::size::Size;
use crate::style::{Stream, Theme};

/// Animated spinner.
pub type Spinner = spinner::Spinner;

/// Ordered multi-step progress display.
pub type Steps = steps::Steps;

/// Cloneable handle for updating a running [`Steps`].
pub type StepsHandle = steps::StepsHandle;

/// Live widget options.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Whether to animate. False for pipes, files, and `TERM=dumb`.
    pub tty: bool,
    /// Color and role policy.
    pub theme: Theme,
}

impl Options {
    /// Detects options for `stream`.
    #[must_use]
    pub fn detect(stream: Stream) -> Self {
        Self {
            tty: stream.is_terminal(),
            theme: Theme::detect(stream),
        }
    }
}

pub(crate) fn surface_size(stream: Stream) -> Size {
    surface_size_of(stream)
}

#[cfg(feature = "size")]
fn surface_size_of(stream: Stream) -> Size {
    match stream {
        Stream::Stdout => crate::size::stdout(),
        Stream::Stderr => crate::size::stderr(),
    }
}

#[cfg(not(feature = "size"))]
fn surface_size_of(_stream: Stream) -> Size {
    Size::default()
}
