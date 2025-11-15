#[derive(Debug, Clone, PartialEq)]
pub enum NetworkState {
    Uninitialized,
    Initializing,
    Ready,
    Degraded,
    Failed,
}

impl std::fmt::Display for NetworkState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetworkState::Uninitialized => write!(f, "uninitialized"),
            NetworkState::Initializing => write!(f, "initializing"),
            NetworkState::Ready => write!(f, "ready"),
            NetworkState::Degraded => write!(f, "degraded"),
            NetworkState::Failed => write!(f, "failed"),
        }
    }
}
