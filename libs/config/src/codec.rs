//! Format-agnostic serialization codec abstraction.

use serde::{Serialize, de::DeserializeOwned};

use crate::error::Result;

pub(crate) trait Codec {
    fn encode<T: Serialize>(value: &T) -> Result<String>;
    fn decode<T: DeserializeOwned>(s: &str) -> Result<T>;
}

pub(crate) struct TomlCodec;

impl Codec for TomlCodec {
    fn encode<T: Serialize>(value: &T) -> Result<String> {
        toml::to_string_pretty(value).map_err(Into::into)
    }

    fn decode<T: DeserializeOwned>(s: &str) -> Result<T> {
        toml::from_str(s).map_err(Into::into)
    }
}
