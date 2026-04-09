//! Runtime helpers provided to granola-supervised services.

mod notify;
mod signal;
mod socket;

pub use notify::{Health, NotifyClient};
pub use signal::shutdown_signal;
pub use socket::socket;
