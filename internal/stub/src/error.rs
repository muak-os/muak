use core::fmt;
use uefi::Status;

pub type StubResult<T> = Result<T, StubError>;

#[derive(Debug, Clone, Copy)]
pub enum StubError {
    InvalidDosMagic,
    NoKernelSection,
    AllocationFailed,
    ProtocolInstallFailed,
    ProtocolOpenFailed,
    KernelLoadFailed,
}

impl StubError {
    pub fn to_status(self) -> Status {
        match self {
            StubError::InvalidDosMagic => Status::LOAD_ERROR,
            StubError::NoKernelSection => Status::NOT_FOUND,
            StubError::AllocationFailed => Status::OUT_OF_RESOURCES,
            StubError::ProtocolInstallFailed => Status::PROTOCOL_ERROR,
            StubError::ProtocolOpenFailed => Status::PROTOCOL_ERROR,
            StubError::KernelLoadFailed => Status::LOAD_ERROR,
        }
    }
}

impl fmt::Display for StubError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StubError::InvalidDosMagic => write!(f, "invalid DOS header magic"),
            StubError::NoKernelSection => write!(f, "no .linux section found"),
            StubError::AllocationFailed => write!(f, "memory allocation failed"),
            StubError::ProtocolInstallFailed => write!(f, "failed to install protocol"),
            StubError::ProtocolOpenFailed => write!(f, "failed to open protocol"),
            StubError::KernelLoadFailed => write!(f, "failed to load kernel image"),
        }
    }
}
