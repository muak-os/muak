use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs};
use std::time::{Duration, Instant, SystemTime};

use anyhow::Result;
use tokio::time::timeout;

use super::config::{
    CONNECTIVITY_CHECK_INTERVAL_SECS, CONNECTIVITY_OVERALL_TIMEOUT_SECS,
    CONNECTIVITY_PROBE_TIMEOUT_SECS,
};
use super::model::{ConnectivityResult, ConnectivityStatus};
use crate::log;

#[derive(Debug, Clone)]
pub struct ConnectivityConfig {
    pub target: ConnectivityTarget,
    pub probe_timeout: Duration,
    pub overall_timeout: Duration,
    pub check_interval: Duration,
}

impl Default for ConnectivityConfig {
    fn default() -> Self {
        Self {
            target: ConnectivityTarget {
                host: "leomercier.dev".to_string(),
            },
            probe_timeout: Duration::from_secs(CONNECTIVITY_PROBE_TIMEOUT_SECS),
            overall_timeout: Duration::from_secs(CONNECTIVITY_OVERALL_TIMEOUT_SECS),
            check_interval: Duration::from_secs(CONNECTIVITY_CHECK_INTERVAL_SECS),
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
            log!("network", "Connectivity check timed out");
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
            log!("network", "DNS failed for {}: {}", target.host, e);
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
            log!("network", "HTTPS check failed for {}: {}", target.host, e);
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
        anyhow::bail!("unexpected status: {}", response.status())
    }
}
