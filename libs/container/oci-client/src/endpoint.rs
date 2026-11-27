//! Registry URL scheme selection.

use core::net::IpAddr;

use oci::reference::Image;

/// Determine the URL scheme (`http` or `https`) for the registry.
pub(crate) fn scheme(image: &Image) -> &'static str {
    if is_plaintext(&image.registry) {
        "http"
    } else {
        "https"
    }
}

fn is_plaintext(registry: &str) -> bool {
    if registry == "localhost" || registry.starts_with("localhost:") {
        return true;
    }
    match host_addr(registry) {
        Some(addr) => addr.is_loopback() || matches!(addr, IpAddr::V4(addr) if addr.is_private()),
        None => false,
    }
}

fn host_addr(registry: &str) -> Option<IpAddr> {
    if let Some((host, _)) = registry.rsplit_once(':')
        && let Ok(addr) = host.parse::<IpAddr>()
    {
        return Some(addr);
    }
    registry.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_for_docker_io() {
        // ARRANGE
        let reference = "alpine:latest";

        // ACT
        let image = Image::parse(reference);

        // ASSERT
        assert_eq!(scheme(&image), "https");
    }

    #[test]
    fn http_for_private() {
        // ARRANGE
        let reference = "192.168.1.1:5000/image:tag";

        // ACT
        let image = Image::parse(reference);

        // ASSERT
        assert_eq!(scheme(&image), "http");
    }

    #[test]
    fn http_for_localhost_registry() {
        // ARRANGE
        let reference = "localhost:5000/repo:tag";

        // ACT
        let image = Image::parse(reference);

        // ASSERT
        assert_eq!(scheme(&image), "http");
    }

    #[test]
    fn https_for_public_172_registry() {
        // ARRANGE
        let reference = "172.104.1.1:5000/repo:tag";

        // ACT
        let image = Image::parse(reference);

        // ASSERT
        assert_eq!(scheme(&image), "https");
    }

    #[test]
    fn http_for_private_172_registry() {
        // ARRANGE
        let reference = "172.16.0.1:5000/repo:tag";

        // ACT
        let image = Image::parse(reference);

        // ASSERT
        assert_eq!(scheme(&image), "http");
    }

    #[test]
    fn http_for_ten_range_registry() {
        // ARRANGE
        let reference = "10.0.0.1:5000/repo:tag";

        // ACT
        let image = Image::parse(reference);

        // ASSERT
        assert_eq!(scheme(&image), "http");
    }

    #[test]
    fn http_for_loopback_registry() {
        // ARRANGE
        let reference = "127.0.0.1:5000/repo:tag";

        // ACT
        let image = Image::parse(reference);

        // ASSERT
        assert_eq!(scheme(&image), "http");
    }
}
