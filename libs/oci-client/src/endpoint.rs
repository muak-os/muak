//! Registry URL scheme selection.

use oci::reference::Image;

/// Determine the URL scheme (`http` or `https`) for the registry.
pub(crate) fn scheme(image: &Image) -> &'static str {
    if image.registry.starts_with("192.168.")
        || image.registry.starts_with("10.")
        || image.registry.starts_with("172.")
        || image.registry.starts_with("127.")
        || image.registry.starts_with("localhost")
    {
        "http"
    } else {
        "https"
    }
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
}
