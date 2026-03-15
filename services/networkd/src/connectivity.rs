use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Result, bail};
use tokio::time::timeout;

use crate::model::{ConnectivityResult, ConnectivityStatus};

/// Timeout for each individual probe.
const PROBE_TIMEOUT_SECS: u64 = 5;

/// Overall timeout for the entire connectivity check process.
const OVERALL_TIMEOUT_SECS: u64 = 15;

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

async fn check_target(target: &ConnectivityTarget, probe_timeout: Duration) -> ConnectivityResult {
    let mut result = ConnectivityResult {
        status: ConnectivityStatus::Checking,
        last_check: SystemTime::now(),
        ..Default::default()
    };

    match resolve_dns(&target.host, probe_timeout).await {
        Ok(_) => {
            result.dns_ok = true;
        }
        Err(e) => {
            kmsg::warn!("DNS failed for {}: {}", target.host, e);
            result.status = ConnectivityStatus::Disconnected;
            result.last_check = SystemTime::now();
            return result;
        }
    };

    match https_check(&target.host, probe_timeout).await {
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

async fn resolve_dns(host: &str, timeout_dur: Duration) -> Result<Ipv4Addr> {
    let host = host.to_string();
    timeout(
        timeout_dur,
        tokio::task::spawn_blocking(move || {
            let addrs: Vec<_> = format!("{}:443", host).to_socket_addrs()?.collect();
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

async fn https_check(host: &str, timeout_dur: Duration) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(timeout_dur)
        .local_address(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
        .build()?;

    let url = format!("https://{}/", host);
    let response = client.get(&url).send().await?;

    if response.status().is_success() || response.status().is_redirection() {
        Ok(())
    } else {
        bail!("unexpected status: {}", response.status())
    }
}
