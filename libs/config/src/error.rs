use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    ReadError(#[from] std::io::Error),

    #[error("Failed to parse config: {0}")]
    ParseError(#[from] toml::de::Error),

    #[error("Failed to serialize config: {0}")]
    SerializeError(#[from] toml::ser::Error),

    #[error("Config validation failed: {0}")]
    ValidationError(String),

    #[error("Config already initialized")]
    AlreadyInitialized,

    #[error("Config not initialized - call config::init() first")]
    NotInitialized,
}

pub type Result<T> = std::result::Result<T, ConfigError>;
