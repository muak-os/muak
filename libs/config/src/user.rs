//! Client configuration for managing multiple server contexts.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use base64ct::{Base64, Encoding};
use serde::{Deserialize, Serialize};

use crate::codec::{Codec, TomlCodec};

const CONFIG_FILE: &str = "config.toml";

/// Decoded credentials: (CA, certificate, key).
pub type Credentials = (Vec<u8>, Vec<u8>, Vec<u8>);

/// Client configuration storing multiple server contexts.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClientConfig {
    /// Name of the currently active context.
    pub context: Option<String>,
    /// Map of named server contexts.
    #[serde(default)]
    pub contexts: HashMap<String, ServerContext>,
    /// Map of pending enrollments by endpoint.
    #[serde(default)]
    pub pending: HashMap<String, PendingEnrollment>,
}

/// A server context containing endpoint and credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerContext {
    /// Server endpoint address (host:port).
    pub endpoint: String,
    /// Base64-encoded CA certificate.
    pub ca: Option<String>,
    /// Base64-encoded client certificate.
    pub crt: Option<String>,
    /// Base64-encoded client private key.
    pub key: Option<String>,
}

/// A pending enrollment waiting for admin approval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingEnrollment {
    /// Client certificate fingerprint for this enrollment.
    pub fingerprint: String,
    /// Base64-encoded client key pair.
    pub key: String,
    /// Server certificate fingerprint from the initial connection.
    pub server_fingerprint: String,
}

fn config_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME environment variable not set")?;
    Ok(PathBuf::from(home).join(".config/muak"))
}

fn config_file_path() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("MUAK_CONFIG") {
        return Ok(PathBuf::from(path));
    }
    Ok(config_dir()?.join(CONFIG_FILE))
}

impl ClientConfig {
    /// Load config from disk. Returns empty config if file doesn't exist.
    pub fn load() -> Result<Self> {
        let path = config_file_path()?;

        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config from {:?}", path))?;

        TomlCodec::decode(&contents)
            .with_context(|| format!("Failed to parse config from {:?}", path))
    }

    /// Save config to disk.
    pub fn save(&self) -> Result<()> {
        let path = config_file_path()?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create config directory {:?}", parent))?;
        }

        let contents = TomlCodec::encode(self).context("Failed to serialize config")?;

        std::fs::write(&path, contents)
            .with_context(|| format!("Failed to write config to {:?}", path))
    }

    /// Get the currently active context name and data.
    pub fn current_context(&self) -> Option<(&str, &ServerContext)> {
        self.context
            .as_ref()
            .and_then(|name| self.contexts.get(name).map(|ctx| (name.as_str(), ctx)))
    }

    /// Get a context by name.
    pub fn get_context(&self, name: &str) -> Option<&ServerContext> {
        self.contexts.get(name)
    }

    /// Add a new context, handling name collisions.
    pub fn add_context(&mut self, name: &str, ctx: ServerContext) -> String {
        let actual_name = resolve_name_collision(&self.contexts, name);
        self.contexts.insert(actual_name.clone(), ctx);
        actual_name
    }

    /// Remove a context by name.
    pub fn remove_context(&mut self, name: &str) -> Result<()> {
        if !self.contexts.contains_key(name) {
            bail!("Context '{}' not found", name);
        }

        self.contexts.remove(name);

        if self.context.as_deref() == Some(name) {
            self.context = None;
        }

        Ok(())
    }

    /// Set the current context.
    pub fn set_current(&mut self, name: &str) -> Result<()> {
        if !self.contexts.contains_key(name) {
            bail!(
                "Context '{}' not found. Run 'muakctl context list' to see available contexts.",
                name
            );
        }
        self.context = Some(name.to_string());
        Ok(())
    }

    /// List all context names.
    pub fn list_contexts(&self) -> Vec<&str> {
        let mut names: Vec<_> = self.contexts.keys().map(String::as_str).collect();
        names.sort();
        names
    }

    /// Check if credentials exist for the given endpoint.
    pub fn has_credentials_for_endpoint(&self, endpoint: &str) -> bool {
        self.contexts
            .values()
            .any(|ctx| ctx.endpoint == endpoint && ctx.has_credentials())
    }

    /// Starts an enrollment by saving the pending state.
    pub fn start_enrollment(
        &mut self,
        endpoint: &str,
        key_pem: &str,
        fingerprint: &str,
        server_fingerprint: &str,
    ) {
        self.pending.insert(
            endpoint.to_string(),
            PendingEnrollment {
                fingerprint: fingerprint.to_string(),
                key: Base64::encode_string(key_pem.as_bytes()),
                server_fingerprint: server_fingerprint.to_string(),
            },
        );
    }

    /// Gets a pending enrollment for the given endpoint.
    pub fn get_pending(&self, endpoint: &str) -> Option<&PendingEnrollment> {
        self.pending.get(endpoint)
    }

    /// Completes an enrollment by creating a context and clearing pending state.
    pub fn complete_enrollment(
        &mut self,
        endpoint: &str,
        server_name: &str,
        ca_pem: &str,
        cert_pem: &str,
        key_pem: &[u8],
    ) -> String {
        let ctx = ServerContext::from_pem(endpoint, ca_pem, cert_pem, key_pem);
        let name = self.add_context(server_name, ctx);
        self.pending.remove(endpoint);
        self.context = Some(name.clone());
        name
    }

    /// Cancels an enrollment by removing the pending state.
    pub fn cancel_enrollment(&mut self, endpoint: &str) {
        self.pending.remove(endpoint);
    }
}

impl ServerContext {
    /// Create a new context from PEM strings.
    pub fn from_pem(endpoint: &str, ca: &str, crt: &str, key: &[u8]) -> Self {
        Self {
            endpoint: endpoint.to_string(),
            ca: Some(Base64::encode_string(ca.as_bytes())),
            crt: Some(Base64::encode_string(crt.as_bytes())),
            key: Some(Base64::encode_string(key)),
        }
    }

    /// Decode credentials from base64.
    pub fn credentials(&self) -> Result<Option<Credentials>> {
        let (ca, crt, key) = match (&self.ca, &self.crt, &self.key) {
            (Some(ca), Some(crt), Some(key)) => (ca, crt, key),
            _ => return Ok(None),
        };

        let ca_bytes = Base64::decode_vec(ca).context("Failed to decode CA certificate")?;
        let crt_bytes = Base64::decode_vec(crt).context("Failed to decode client certificate")?;
        let key_bytes = Base64::decode_vec(key).context("Failed to decode client key")?;

        Ok(Some((ca_bytes, crt_bytes, key_bytes)))
    }

    /// Check if this context has credentials.
    pub fn has_credentials(&self) -> bool {
        self.ca.is_some() && self.crt.is_some() && self.key.is_some()
    }
}

fn resolve_name_collision(existing: &HashMap<String, ServerContext>, base_name: &str) -> String {
    if !existing.contains_key(base_name) {
        return base_name.to_string();
    }

    format!(
        "{}-{}",
        base_name,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_secs()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config() {
        // ACT
        let config = ClientConfig::default();

        // ASSERT
        assert!(config.context.is_none());
        assert!(config.contexts.is_empty());
    }

    #[test]
    fn add_context() {
        // ARRANGE
        let mut config = ClientConfig::default();
        let ctx = ServerContext {
            endpoint: "localhost:50051".to_string(),
            ca: None,
            crt: None,
            key: None,
        };

        // ACT
        let name = config.add_context("test", ctx);

        // ASSERT
        assert_eq!(name, "test");
        assert!(config.contexts.contains_key("test"));
    }

    #[test]
    fn set_current() {
        // ARRANGE
        let mut config = ClientConfig::default();
        let ctx = ServerContext {
            endpoint: "localhost:50051".to_string(),
            ca: None,
            crt: None,
            key: None,
        };

        // ACT
        config.add_context("test", ctx);
        assert!(config.set_current("test").is_ok());

        // ASSERT
        assert_eq!(config.context.as_deref(), Some("test"));
    }

    #[test]
    fn set_current_not_found() {
        // ARRANGE
        let mut config = ClientConfig::default();

        // ACT & ASSERT
        assert!(config.set_current("nonexistent").is_err());
    }

    #[test]
    fn remove_context() {
        // ARRANGE
        let mut config = ClientConfig::default();
        let ctx = ServerContext {
            endpoint: "localhost:50051".to_string(),
            ca: None,
            crt: None,
            key: None,
        };

        config.add_context("test", ctx);
        config.set_current("test").unwrap();

        // ACT
        assert!(config.remove_context("test").is_ok());

        // ASSERT
        assert!(config.contexts.is_empty());
        assert!(config.context.is_none());
    }

    #[test]
    fn credentials_encoding() {
        // ARRANGE
        let ctx = ServerContext::from_pem("localhost:50051", "ca-data", "crt-data", b"key-data");

        // ACT
        let creds = ctx.credentials().unwrap().unwrap();

        // ASSERT
        assert!(ctx.has_credentials());
        assert_eq!(creds.0, b"ca-data");
        assert_eq!(creds.1, b"crt-data");
        assert_eq!(creds.2, b"key-data");
    }

    #[test]
    fn has_credentials_for_endpoint() {
        // ARRANGE
        let mut config = ClientConfig::default();

        let ctx1 = ServerContext {
            endpoint: "server1:50051".to_string(),
            ca: None,
            crt: None,
            key: None,
        };
        config.add_context("no-creds", ctx1);

        let ctx2 = ServerContext::from_pem("server2:50051", "ca", "crt", b"key");
        config.add_context("with-creds", ctx2);

        // ACT & ASSERT
        assert!(!config.has_credentials_for_endpoint("server1:50051"));
        assert!(config.has_credentials_for_endpoint("server2:50051"));
        assert!(!config.has_credentials_for_endpoint("unknown:50051"));
    }

    #[test]
    fn current_context() {
        // ARRANGE
        let mut config = ClientConfig::default();

        // ACT
        let ctx = ServerContext::from_pem("localhost:50051", "ca", "crt", b"key");
        config.add_context("test", ctx);
        config.set_current("test").unwrap();

        // ASSERT
        assert!(config.current_context().is_some());
        let (name, ctx) = config.current_context().unwrap();
        assert_eq!(name, "test");
        assert_eq!(ctx.endpoint, "localhost:50051");
    }

    #[test]
    fn list_contexts_sorted() {
        // ARRANGE
        let mut config = ClientConfig::default();

        config.add_context(
            "zebra",
            ServerContext {
                endpoint: "z:50051".to_string(),
                ca: None,
                crt: None,
                key: None,
            },
        );
        config.add_context(
            "alpha",
            ServerContext {
                endpoint: "a:50051".to_string(),
                ca: None,
                crt: None,
                key: None,
            },
        );

        // ACT
        let names = config.list_contexts();

        // ASSERT
        assert_eq!(names, vec!["alpha", "zebra"]);
    }

    #[test]
    fn load_save_via_muak_config() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let path_str = path.to_str().unwrap().to_string();

        // SAFETY: single-threaded test; no other threads read env at this point
        unsafe { std::env::set_var("MUAK_CONFIG", &path_str) };

        let config = ClientConfig::load().unwrap();
        assert!(config.contexts.is_empty());

        // ACT
        let mut config = ClientConfig::default();
        config.add_context(
            "prod",
            ServerContext::from_pem("prod:50051", "ca", "crt", b"key"),
        );
        config.set_current("prod").unwrap();
        config.save().unwrap();

        let loaded = ClientConfig::load().unwrap();

        // ASSERT
        assert_eq!(loaded.context.as_deref(), Some("prod"));
        assert!(loaded.contexts.contains_key("prod"));

        // SAFETY: single-threaded test; no other threads read env at this point
        unsafe { std::env::remove_var("MUAK_CONFIG") };
    }

    #[test]
    fn get_context() {
        // ARRANGE
        let mut config = ClientConfig::default();

        // ACT & ASSERT
        assert!(config.get_context("missing").is_none());

        config.add_context(
            "srv",
            ServerContext {
                endpoint: "srv:50051".to_string(),
                ca: None,
                crt: None,
                key: None,
            },
        );
        let ctx = config.get_context("srv").unwrap();
        assert_eq!(ctx.endpoint, "srv:50051");
    }

    #[test]
    fn remove_context_not_found() {
        // ARRANGE
        let mut config = ClientConfig::default();

        // ACT & ASSERT
        assert!(config.remove_context("ghost").is_err());
    }

    #[test]
    fn remove_non_current_context() {
        // ARRANGE
        let mut config = ClientConfig::default();
        config.add_context(
            "a",
            ServerContext {
                endpoint: "a:50051".to_string(),
                ca: None,
                crt: None,
                key: None,
            },
        );
        config.add_context(
            "b",
            ServerContext {
                endpoint: "b:50051".to_string(),
                ca: None,
                crt: None,
                key: None,
            },
        );
        config.set_current("a").unwrap();

        // ACT
        config.remove_context("b").unwrap();

        // ASSERT
        assert_eq!(config.context.as_deref(), Some("a"));
        assert!(!config.contexts.contains_key("b"));
    }

    #[test]
    fn credentials_none_when_fields_missing() {
        // ARRANGE
        let ctx = ServerContext {
            endpoint: "x:50051".to_string(),
            ca: None,
            crt: None,
            key: None,
        };

        // ACT
        let creds = ctx.credentials().unwrap();

        // ASSERT
        assert!(!ctx.has_credentials());
        assert!(creds.is_none());
    }

    #[test]
    fn resolve_name_collision_deduplicates() {
        // ARRANGE
        let mut map: HashMap<String, ServerContext> = HashMap::new();

        // ACT
        let name = resolve_name_collision(&map, "server");

        // ASSERT
        assert_eq!(name, "server");

        map.insert(
            "server".to_string(),
            ServerContext {
                endpoint: "x:50051".to_string(),
                ca: None,
                crt: None,
                key: None,
            },
        );

        // ACT
        let name2 = resolve_name_collision(&map, "server");

        // ASSERT
        assert!(name2.starts_with("server-"));
        assert_ne!(name2, "server");
    }

    #[test]
    fn enrollment_lifecycle() {
        // ARRANGE
        let mut config = ClientConfig::default();

        // ACT
        config.start_enrollment("https://server:443", "key-pem", "fp123", "server-fp456");
        let pending = config.get_pending("https://server:443").unwrap();

        // ASSERT
        assert_eq!(pending.fingerprint, "fp123");
        assert_eq!(pending.server_fingerprint, "server-fp456");

        // ACT
        let name =
            config.complete_enrollment("https://server:443", "myserver", "ca", "cert", b"key");

        // ASSERT
        assert_eq!(config.context.as_deref(), Some(&name as &str));
        assert!(config.get_pending("https://server:443").is_none());
        assert!(config.contexts.contains_key(&name));
    }

    #[test]
    fn cancel_enrollment() {
        // ARRANGE
        let mut config = ClientConfig::default();

        // ACT
        config.start_enrollment("https://server:443", "key", "fp", "sfp");

        // ASSERT
        assert!(config.get_pending("https://server:443").is_some());

        // ACT
        config.cancel_enrollment("https://server:443");

        // ASSERT
        assert!(config.get_pending("https://server:443").is_none());
    }

    #[test]
    fn load_invalid_toml_returns_error() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "not valid toml ][[[").unwrap();

        // SAFETY: single-threaded test; no other threads read env at this point
        unsafe { std::env::set_var("MUAK_CONFIG", path.to_str().unwrap()) };

        // ACT
        let result = ClientConfig::load();

        // ASSERT
        assert!(result.is_err());

        // SAFETY: same as above
        unsafe { std::env::remove_var("MUAK_CONFIG") };
    }
}
