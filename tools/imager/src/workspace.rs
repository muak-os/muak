//! Temporary and working directory management.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Creates a unique workspace directory inside the given parent.
pub fn unique(parent: &Path) -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    parent.join(format!(".work-{}.{}", ts.as_secs(), ts.subsec_nanos()))
}
