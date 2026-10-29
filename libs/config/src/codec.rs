//! Format-agnostic serialization codec abstraction.

use serde::{Serialize, de::DeserializeOwned};

use crate::error::Result;

pub(crate) trait Codec {
    fn decode<T: DeserializeOwned>(contents: &str) -> Result<T>;
    fn encode<T: Serialize>(value: &T) -> Result<String>;
}

pub(crate) struct TomlCodec;

impl Codec for TomlCodec {
    fn encode<T: Serialize>(value: &T) -> Result<String> {
        toml::to_string_pretty(value).map_err(Into::into)
    }

    fn decode<T: DeserializeOwned>(contents: &str) -> Result<T> {
        toml::from_str(contents).map_err(Into::into)
    }
}
