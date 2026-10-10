//! HTTP request builders for registry operations.

use http_body_util::Full;
use hyper::body::Bytes;
use hyper::http::request::Builder;
use hyper::{Method, Request};

use crate::error::{ClientError, Result};
use crate::http::Range;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.3";

pub(crate) fn get_request(
    url: &str,
    authorization: Option<&str>,
    accept_headers: &[&str],
    range: Option<&Range>,
) -> Result<Request<Full<Bytes>>> {
    let mut builder = base_request(Method::GET, url);
    for accept in accept_headers {
        builder = builder.header("Accept", *accept);
    }
    if let Some(range) = range {
        builder = builder.header(hyper::header::RANGE, range.header_value());
    }

    finish_request(builder, authorization, Full::new(Bytes::new()))
}

pub(crate) fn head_request(url: &str, authorization: Option<&str>) -> Result<Request<Full<Bytes>>> {
    finish_request(
        base_request(Method::HEAD, url),
        authorization,
        Full::new(Bytes::new()),
    )
}

pub(crate) fn post_request(url: &str, authorization: Option<&str>) -> Result<Request<Full<Bytes>>> {
    finish_request(
        base_request(Method::POST, url),
        authorization,
        Full::new(Bytes::new()),
    )
}

pub(crate) fn put_request(
    url: &str,
    authorization: Option<&str>,
    content_type: &str,
    body: Bytes,
) -> Result<Request<Full<Bytes>>> {
    let builder = base_request(Method::PUT, url).header("Content-Type", content_type);

    finish_request(builder, authorization, Full::new(body))
}

pub(crate) fn patch_request(
    url: &str,
    authorization: Option<&str>,
    content_type: &str,
    content_range: &str,
    content_length: usize,
    body: Bytes,
) -> Result<Request<Full<Bytes>>> {
    let builder = base_request(Method::PATCH, url)
        .header("Content-Type", content_type)
        .header("Content-Range", content_range)
        .header("Content-Length", content_length);

    finish_request(builder, authorization, Full::new(body))
}

fn base_request(method: Method, url: &str) -> Builder {
    Request::builder()
        .method(method)
        .uri(url)
        .header("User-Agent", USER_AGENT)
}

fn finish_request(
    builder: Builder,
    authorization: Option<&str>,
    body: Full<Bytes>,
) -> Result<Request<Full<Bytes>>> {
    let builder = match authorization {
        Some(value) => builder.header("Authorization", value),
        None => builder,
    };

    builder
        .body(body)
        .map_err(|error| ClientError::Network(format!("Failed to build request: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_requests_carry_accept_and_range_headers() {
        // ARRANGE
        let range = Range::from_start(2048);

        // ACT
        let request = get_request(
            "http://registry/v2/x",
            Some("token"),
            &["application/json"],
            Some(&range),
        )
        .expect("build request");

        // ASSERT
        assert_eq!(
            request
                .headers()
                .get("Accept")
                .map(|value| value.to_str().expect("utf8")),
            Some("application/json")
        );
        assert_eq!(
            request
                .headers()
                .get(hyper::header::RANGE)
                .map(|value| value.to_str().expect("utf8")),
            Some("bytes=2048-")
        );
        assert_eq!(
            request
                .headers()
                .get(hyper::header::AUTHORIZATION)
                .map(|value| value.to_str().expect("utf8")),
            Some("token")
        );
    }

    #[test]
    fn put_requests_carry_the_content_type_and_body() {
        // ARRANGE
        let body = Bytes::from_static(b"payload");

        // ACT
        let request = put_request(
            "http://registry/v2/x",
            None,
            "application/octet-stream",
            body,
        )
        .expect("build request");

        // ASSERT
        assert_eq!(
            request
                .headers()
                .get("Content-Type")
                .map(|value| value.to_str().expect("utf8")),
            Some("application/octet-stream")
        );
    }

    #[test]
    fn finish_request_rejects_invalid_uris() {
        // ARRANGE

        // ACT
        let error = get_request("http://127.0.0.1:5000/has space", None, &[], None)
            .expect_err("request should fail");

        // ASSERT
        assert!(matches!(error, ClientError::Network(_)));
    }
}
