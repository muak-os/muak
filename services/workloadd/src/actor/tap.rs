//! TAP device setup and teardown for VM networking.

use std::io::{Error, ErrorKind};
use std::time::Duration;

use anyhow::Context as _;
use net_runtime::Link as _;
use net_runtime_linux::Tokio;
use netmodel::Name;

const DELETE_ATTEMPTS: usize = 20;
const DELETE_RETRY_DELAY: Duration = Duration::from_millis(100);

pub(super) async fn setup(
    runtime: &Tokio,
    tap_name: &str,
    bridge_name: &str,
) -> anyhow::Result<()> {
    let (tap, bridge) = names(tap_name, bridge_name)?;

    let queue = tap_linux::create(tap.as_str()).context("failed to create TAP device")?;
    let configured = configure(runtime, &tap, &bridge).await;
    drop(queue);

    if let Err(error) = configured {
        remove(tap.as_str()).await;
        return Err(error);
    }

    Ok(())
}

pub(super) async fn remove(tap_name: &str) {
    for attempt in 1..=DELETE_ATTEMPTS {
        match tap_linux::delete(tap_name) {
            Ok(()) => return,
            Err(error) if should_retry(&error, attempt) => {
                tokio::time::sleep(DELETE_RETRY_DELAY).await;
            }
            Err(error) => {
                eprintln!("Failed to delete TAP device {tap_name}: {error}");
                return;
            }
        }
    }
}

async fn configure(runtime: &Tokio, tap: &Name, bridge: &Name) -> anyhow::Result<()> {
    runtime
        .set_up(tap)
        .await
        .context("failed to bring TAP device up")?;
    runtime
        .set_master(tap, bridge)
        .await
        .context("failed to attach TAP device to bridge")?;

    Ok(())
}

fn should_retry(error: &Error, attempt: usize) -> bool {
    error.kind() == ErrorKind::ResourceBusy && attempt < DELETE_ATTEMPTS
}

fn names(tap_name: &str, bridge_name: &str) -> anyhow::Result<(Name, Name)> {
    let tap = Name::new(tap_name).context("invalid TAP device name")?;
    let bridge = Name::new(bridge_name).context("invalid bridge name")?;

    Ok((tap, bridge))
}

#[cfg(test)]
mod tests {
    use super::names;

    #[test]
    fn names_rejects_empty_tap_name() {
        // ACT
        let result = names("", "br0");

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn names_rejects_too_long_bridge_name() {
        // ARRANGE
        let bridge = "b".repeat(16);

        // ACT
        let result = names("tap-12345678", &bridge);

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn names_accepts_valid_pair() {
        // ACT
        let (tap, bridge) = names("tap-12345678", "br0").expect("valid names");

        // ASSERT
        assert_eq!(tap.as_str(), "tap-12345678");
        assert_eq!(bridge.as_str(), "br0");
    }
}
