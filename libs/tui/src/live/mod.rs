//! Live single-line widgets.

mod lines;
mod renderer;
mod spinner;
mod steps;
pub mod summary;

use crate::glyphs::Glyphs;
use crate::output::{Flags, Mode, Output};
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
    /// Whether to animate. False for plain and quiet output.
    pub tty: bool,
    /// Color and role policy.
    pub theme: Theme,
    /// Rendering mode.
    pub mode: Mode,
    /// Symbol set matching the mode.
    pub glyphs: Glyphs,
    /// Terminal surface dimensions.
    pub size: Size,
    /// Whether per-item detail is enabled.
    pub verbose: bool,
}

impl Options {
    /// Creates options for an explicitly chosen terminal and theme.
    #[must_use]
    pub const fn new(tty: bool, theme: Theme) -> Self {
        if tty {
            Self {
                tty,
                theme,
                mode: Mode::Rich,
                glyphs: Glyphs::rich(),
                size: Size::new(80, 25),
                verbose: false,
            }
        } else {
            Self {
                tty,
                theme,
                mode: Mode::Plain,
                glyphs: Glyphs::plain(),
                size: Size::new(80, 25),
                verbose: false,
            }
        }
    }

    /// Detects options for `stream`.
    #[must_use]
    pub fn detect(stream: Stream) -> Self {
        Self::from_output(Output::detect(stream, Flags::default()))
    }

    /// Builds options from resolved `output`.
    #[must_use]
    pub const fn from_output(output: Output) -> Self {
        Self {
            tty: output.animates(),
            theme: output.theme,
            mode: output.mode,
            glyphs: output.glyphs,
            size: output.size,
            verbose: output.verbose,
        }
    }
}
