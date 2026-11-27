//! Integration test for reading a response body off the wire.

mod common;

#[cfg(test)]
mod tests {
    use oci_client::http::{collect_body, get_any_status};
    use oci_client::transport::build_client;

    use crate::common;

    #[tokio::test]
    async fn body_read_failure_names_the_request_url() {
        // ARRANGE
        let client = build_client();
        let url = common::spawn_truncated_body().expect("spawn truncated body server");
        let response = get_any_status(&client, &url, None, &[], None)
            .await
            .expect("response head should arrive");

        // ACT
        let error = collect_body(response, &url)
            .await
            .expect_err("truncated body should fail");

        // ASSERT
        assert!(
            error.to_string().contains(&format!("for URL: {url}")),
            "body read error must name the URL: {error}"
        );
    }
}
