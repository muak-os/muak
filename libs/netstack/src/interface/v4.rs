//! IPv4 address method driven by the interface controller.

use core::future::Future;

use crate::{provision, view};

/// IPv4 address method the controller drives.
pub enum Method {
    /// Static address method.
    Static(provision::v4::Static),
}

impl Method {
    /// Creates the method matching the given configuration.
    #[must_use = "the method owns the provisioning state"]
    pub const fn from_config(config: provision::v4::Config) -> Self {
        Self::Static(provision::v4::Static::new(config))
    }

    /// Returns the next event produced by the method.
    pub fn next_event(&mut self) -> impl Future<Output = provision::Event> + Send {
        match *self {
            Self::Static(ref mut method) => provision::Method::next_event(method),
        }
    }

    /// Applies a command from the controller.
    pub fn on_command(&mut self, command: provision::Command) {
        match *self {
            Self::Static(ref mut method) => provision::Method::on_command(method, command),
        }
    }

    /// Returns the method's current view, or `None` when it has nothing to report.
    #[must_use = "callers publish the view in the interface snapshot"]
    pub fn view(&self) -> Option<view::V4State> {
        match *self {
            Self::Static(ref method) => provision::Method::view(method),
        }
    }
}
