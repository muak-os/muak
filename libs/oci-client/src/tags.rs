//! Registry tag listing.

use hyper::http::header::LINK;
use oci::reference::Image;
use serde::Deserialize;

use crate::client::Client;
use crate::error::{ClientError, Result};
use crate::http::{collect_body, get};

const PAGE_SIZE: usize = 1000;
const TAGS_ACCEPT: &str = "application/json";

/// Build the paginated `tags/list` URL for a repository image.
#[must_use]
pub fn build_url(image: &Image) -> String {
    format!(
        "{}://{}/v2/{}/tags/list?n={PAGE_SIZE}",
        image.scheme(),
        image.registry,
        image.name,
    )
}

/// List every tag of the client's repository, following pagination links.
///
/// # Errors
///
/// Returns an error when a request fails or a response is not a valid tags
/// list.
pub async fn list(client: &Client) -> Result<Vec<String>> {
    let mut url = build_url(client.image());
    let mut tags = Vec::new();

    loop {
        let response = get(client.http(), &url, client.authorization(), &[TAGS_ACCEPT]).await?;
        let next = response
            .headers()
            .get(LINK)
            .and_then(|value| value.to_str().ok())
            .and_then(next_link);
        let body = collect_body(response).await?;
        tags.extend(parse_tags(&body)?);

        let Some(next) = next else {
            return Ok(tags);
        };
        url = next;
    }
}

#[derive(Deserialize)]
struct TagList {
    tags: Option<Vec<String>>,
}

fn parse_tags(body: &[u8]) -> Result<Vec<String>> {
    let list: TagList = serde_json::from_slice(body)
        .map_err(|error| ClientError::Network(format!("Invalid tags list response: {error}")))?;

    Ok(list.tags.unwrap_or_default())
}

fn next_link(link: &str) -> Option<String> {
    link.split(',').find_map(|entry| {
        let (url, params) = entry.trim().split_once(';')?;
        let url = url.trim().strip_prefix('<')?.strip_suffix('>')?;
        params
            .split(';')
            .any(|param| {
                let param = param.trim();
                param == "rel=next" || param == "rel=\"next\""
            })
            .then(|| url.to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tags_reads_the_tag_array() {
        // ARRANGE
        let body = br#"{"name":"muak/linux","tags":["v6.12.4","latest"]}"#;

        // ACT
        let tags = parse_tags(body).expect("valid tags list");

        // ASSERT
        assert_eq!(tags, ["v6.12.4", "latest"]);
    }

    #[test]
    fn parse_tags_defaults_missing_and_null_tag_arrays() {
        // ARRANGE / ACT
        let missing = parse_tags(br#"{"name":"muak/linux"}"#).expect("valid tags list");
        let null = parse_tags(br#"{"name":"muak/linux","tags":null}"#).expect("valid tags list");

        // ASSERT
        assert!(missing.is_empty());
        assert!(null.is_empty());
    }

    #[test]
    fn parse_tags_rejects_non_json_bodies() {
        // ARRANGE
        let body = b"<html>gateway timeout</html>";

        // ACT / ASSERT
        let error = parse_tags(body).expect_err("non-JSON body must fail");
        assert!(matches!(error, ClientError::Network(_)));
    }

    #[test]
    fn next_link_follows_the_rel_next_entry() {
        // ARRANGE
        let link = r#"<https://ghcr.io/v2/muak/linux/tags/list?last=v9&n=1000>; rel="next""#;

        // ACT / ASSERT
        assert_eq!(
            next_link(link),
            Some("https://ghcr.io/v2/muak/linux/tags/list?last=v9&n=1000".to_owned())
        );
    }

    #[test]
    fn next_link_picks_next_among_several_links() {
        // ARRANGE
        let link = r#"<https://ghcr.io/v2/muak/linux/tags/list?last=v3>; rel="prev", <https://ghcr.io/v2/muak/linux/tags/list?last=v9>; rel="next""#;

        // ACT / ASSERT
        assert_eq!(
            next_link(link),
            Some("https://ghcr.io/v2/muak/linux/tags/list?last=v9".to_owned())
        );
    }

    #[test]
    fn next_link_ignores_headers_without_a_next_relation() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(next_link(r#"<https://ghcr.io/v2/x>; rel="prev""#), None);
        assert_eq!(next_link("garbage"), None);
    }

    #[test]
    fn build_url_targets_the_repository_tags_endpoint() {
        // ARRANGE
        let image = Image::parse("localhost:5000/muak/linux:latest");

        // ACT
        let url = build_url(&image);

        // ASSERT
        assert_eq!(
            url,
            format!("http://localhost:5000/v2/muak/linux/tags/list?n={PAGE_SIZE}")
        );
    }
}
