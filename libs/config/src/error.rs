use thiserror::Error;

/// Errors that can occur during config operations.
#[derive(Error, Debug)]
pub enum ConfigError {
    /// Failed to read the config file from disk.
    #[error("Failed to read config file: {0}")]
    ReadError(#[from] std::io::Error),

    /// Failed to parse the config file contents.
    #[error("Failed to parse config: {0}")]
    ParseError(#[from] toml::de::Error),

    /// Failed to serialize the config to a string.
    #[error("Failed to serialize config: {0}")]
    SerializeError(#[from] toml::ser::Error),

    /// Config validation failed with a descriptive message.
    #[error("Config validation failed: {0}")]
    ValidationError(String),

    /// The config document uses a schema version this reader does not know.
    #[error("unsupported config api_version '{found}' (supported: {supported})")]
    UnsupportedVersion {
        /// Schema version carried by the document.
        found: String,
        /// Schema version supported by this reader.
        supported: String,
    },

    /// Config has already been initialized.
    #[error("Config already initialized")]
    AlreadyInitialized,

    /// Config has not been initialized yet.
    #[error("Config not initialized - call config::init() first")]
    NotInitialized,
}

/// Convenience alias for config crate results.
pub type Result<T> = std::result::Result<T, ConfigError>;
