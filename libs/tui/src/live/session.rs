//! Live progress session.

use std::io;

use super::{Steps, StepsHandle};
use crate::output::Output;

/// Owns an optional step renderer and a domain progress reporter.
pub struct Session<R> {
    steps: Option<Steps>,
    live: R,
}

impl<R> Session<R> {
    /// Creates a session, starting a step renderer on `output` when `start_steps`.
    ///
    /// # Errors
    ///
    /// Returns an error when the renderer thread or the reporter factory fails.
    pub fn new<M>(output: Output, start_steps: bool, make: M) -> io::Result<Self>
    where
        M: FnOnce(Output, Option<StepsHandle>) -> io::Result<R>,
    {
        let steps = if start_steps {
            Some(Steps::stderr_with(output)?)
        } else {
            None
        };

        Self::with_steps(output, steps, make)
    }

    /// Creates a session around an already built step renderer.
    ///
    /// # Errors
    ///
    /// Returns an error when the reporter factory fails.
    pub fn with_steps<M>(output: Output, steps: Option<Steps>, make: M) -> io::Result<Self>
    where
        M: FnOnce(Output, Option<StepsHandle>) -> io::Result<R>,
    {
        let handle = steps.as_ref().map(Steps::handle);
        let live = make(output, handle)?;

        Ok(Self { steps, live })
    }

    /// Returns the domain progress reporter.
    #[must_use]
    pub const fn reporter(&self) -> &R {
        &self.live
    }

    /// Stops the step renderer, if any.
    ///
    /// # Errors
    ///
    /// Returns an error when the renderer thread fails.
    pub fn finish(self) -> io::Result<()> {
        match self.steps {
            Some(steps) => steps.finish(),
            None => Ok(()),
        }
    }

    /// Marks the active step as failed, then stops the renderer, if any.
    ///
    /// # Errors
    ///
    /// Returns an error when the renderer thread fails.
    pub fn fail<M: Into<String>>(self, message: M) -> io::Result<()> {
        match self.steps {
            Some(steps) => {
                steps.fail(message);
                steps.finish()
            }
            None => Ok(()),
        }
    }
}
