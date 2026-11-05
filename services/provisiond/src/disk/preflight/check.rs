//! Outcome of one preflight check.

/// Outcome of one install preflight check.
pub(crate) struct Check {
    pub name: &'static str,
    pub passed: bool,
    pub detail: String,
}

impl Check {
    pub(crate) fn pass(name: &'static str, detail: String) -> Self {
        Self {
            name,
            passed: true,
            detail,
        }
    }

    pub(crate) fn fail(name: &'static str, detail: String) -> Self {
        Self {
            name,
            passed: false,
            detail,
        }
    }
}
