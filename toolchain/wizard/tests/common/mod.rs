//! Shared harness for wizard end-to-end tests.

pub mod fixtures;
pub mod pe;
pub mod server;

use std::io::Error as IoError;
use std::sync::OnceLock;

use server::{MockRegistry, Routes};
use tempfile::TempDir;
use wizard::config::{Config, configure};

/// The per-process harness: one registry, one cache, one wizard config.
pub struct Harness {
    pub registry: MockRegistry,
    pub _cache: TempDir,
}

impl Harness {
    pub fn start(routes: Routes) -> Result<Self, IoError> {
        static CONFIGURED: OnceLock<()> = OnceLock::new();

        let registry = MockRegistry::start(routes)?;
        let cache = TempDir::new()?;

        CONFIGURED.get_or_init(|| {
            configure(Config {
                cache_dir: Some(cache.path().to_path_buf()),
                registry: registry.address().to_owned(),
            })
            .expect("configure wizard once per test process");
        });

        Ok(Self {
            registry,
            _cache: cache,
        })
    }
}
