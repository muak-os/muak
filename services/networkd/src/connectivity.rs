use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Result, bail};
use http_body_util::{BodyExt, Empty};
use hyper::body::Bytes;
use hyper::{Method, Request};
use hyper_util::rt::{TokioExecutor, TokioIo};
use rustls::{ClientConfig, RootCertStore};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::pki_types::ServerName;

use crate::model::{ConnectivityResult, ConnectivityStatus};

/// Timeout for each individual probe.
const PROBE_TIMEOUT_SECS: u64 = 5;

/// Overall timeout for the entire connectivity check process.
const OVERALL_TIMEOUT_SECS: u64 = 15;

/// HTTPS port used for connectivity probes.
const HTTPS_PORT: u16 = 443;

#[derive(Debug, Clone)]
pub struct ConnectivityConfig {
    pub target: ConnectivityTarget,
    pub probe_timeout: Duration,
    pub overall_timeout: Duration,
}

impl Default for ConnectivityConfig {
    fn default() -> Self {
        Self::from_network_config()
    }
}

impl ConnectivityConfig {
    /// Build config from the global network configuration, falling back to `muak.dev`.
    pub fn from_network_config() -> Self {
        let host = config::network()
            .connectivity_probe
            .clone()
            .unwrap_or_else(|| "muak.dev".to_string());
        Self {
            target: ConnectivityTarget { host },
            probe_timeout: Duration::from_secs(PROBE_TIMEOUT_SECS),
            overall_timeout: Duration::from_secs(OVERALL_TIMEOUT_SECS),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConnectivityTarget {
    pub host: String,
}

/// Run a full connectivity check and return the result.
pub async fn check_connectivity(config: &ConnectivityConfig) -> ConnectivityResult {
    let start = Instant::now();

    let result = timeout(config.overall_timeout, async {
        check_target(&config.target, config.probe_timeout).await
    })
    .await;

    match result {
        Ok(mut r) => {
            r.latency_ms = Some(start.elapsed().as_millis() as u64);
            r
        }
        Err(_) => {
            kmsg::warn!("Connectivity check timed out");
            ConnectivityResult {
                status: ConnectivityStatus::Disconnected,
                dns_ok: false,
                https_ok: false,
                last_check: SystemTime::now(),
                latency_ms: Some(start.elapsed().as_millis() as u64),
            }
        }
    }
}

/// Probe DNS then HTTPS for the target, returning a partial result on first failure.
async fn check_target(target: &ConnectivityTarget, probe_timeout: Duration) -> ConnectivityResult {
    let mut result = ConnectivityResult {
        status: ConnectivityStatus::Checking,
        last_check: SystemTime::now(),
        ..Default::default()
    };

    let addr = match resolve_dns(&target.host, probe_timeout).await {
        Ok(addr) => {
            result.dns_ok = true;
            addr
        }
        Err(e) => {
            kmsg::warn!("DNS failed for {}: {}", target.host, e);
            result.status = ConnectivityStatus::Disconnected;
            result.last_check = SystemTime::now();
            return result;
        }
    };

    match https_check(&target.host, addr, probe_timeout).await {
        Ok(_) => {
            result.https_ok = true;
            result.status = ConnectivityStatus::Connected;
        }
        Err(e) => {
            kmsg::warn!("HTTPS check failed for {}: {}", target.host, e);
            result.status = ConnectivityStatus::Disconnected;
        }
    }

    result.last_check = SystemTime::now();
    result
}

/// Resolve the host to its first IPv4 address via a blocking DNS lookup.
async fn resolve_dns(host: &str, timeout_dur: Duration) -> Result<Ipv4Addr> {
    let host = host.to_string();
    timeout(
        timeout_dur,
        tokio::task::spawn_blocking(move || {
            let addrs: Vec<_> = format!("{}:{}", host, HTTPS_PORT)
                .to_socket_addrs()?
                .collect();
            addrs
                .iter()
                .find_map(|a| match a.ip() {
                    IpAddr::V4(v4) => Some(v4),
                    IpAddr::V6(_) => None,
                })
                .ok_or_else(|| anyhow::anyhow!("no IPv4 addresses resolved"))
        }),
    )
    .await??
}

/// Build a rustls [`ClientConfig`] trusting the Mozilla CA root bundle.
fn tls_config() -> Arc<ClientConfig> {
    let mut root_store = RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    Arc::new(
        ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth(),
    )
}

/// Perform a single HTTPS GET and accept any 2xx or 3xx response as success.
async fn https_check(host: &str, addr: Ipv4Addr, timeout_dur: Duration) -> Result<()> {
    let connector = TlsConnector::from(tls_config());
    let server_name = ServerName::try_from(host.to_string())?;
    let sock_addr = SocketAddr::new(IpAddr::V4(addr), HTTPS_PORT);

    let tcp = timeout(timeout_dur, TcpStream::connect(sock_addr)).await??;
    let tls = timeout(timeout_dur, connector.connect(server_name, tcp)).await??;

    let io = TokioIo::new(tls);
    let (mut sender, conn) =
        hyper::client::conn::http2::handshake(TokioExecutor::new(), io).await?;
    tokio::spawn(conn);

    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("https://{}/", host))
        .header("Host", host)
        .body(Empty::<Bytes>::new())?;

    let resp = timeout(timeout_dur, sender.send_request(req)).await??;
    let status = resp.status();
    let _ = resp.collect().await?;

    if status.is_success() || status.is_redirection() {
        Ok(())
    } else {
        bail!("unexpected status: {}", status)
    }
}
