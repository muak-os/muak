use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkMode {
    NAT,
    Bridge,
}

impl Default for NetworkMode {
    fn default() -> Self {
        NetworkMode::Bridge
    }
}

impl fmt::Display for NetworkMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetworkMode::NAT => write!(f, "nat"),
            NetworkMode::Bridge => write!(f, "bridge"),
        }
    }
}

impl FromStr for NetworkMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "nat" => Ok(NetworkMode::NAT),
            "bridge" => Ok(NetworkMode::Bridge),
            _ => Err(format!("Invalid network mode: {}", s)),
        }
    }
}
