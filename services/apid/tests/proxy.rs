//! Integration tests for the proxy functionality.

mod fixtures;

#[cfg(test)]
mod tests {
    use apid::proxy::{self, BackendPool};
    use http_body_util::{BodyExt as _, Empty, Full};
    use hyper::body::{Bytes, Incoming};
    use hyper::{Request, Response};

    use super::fixtures::mock_backend::MockBackend;

    fn ok_response_with_body(body: &'static [u8]) -> Response<Full<Bytes>> {
        Response::builder()
            .status(200)
            .header("content-type", "application/grpc")
            .header("grpc-status", "0")
            .body(Full::new(Bytes::from_static(body)))
            .unwrap()
    }

    fn ok_response_with_path(path: &str) -> Response<Full<Bytes>> {
        Response::builder()
            .status(200)
            .header("content-type", "application/grpc")
            .header("grpc-status", "0")
            .header("x-received-path", path)
            .body(Full::new(Bytes::new()))
            .unwrap()
    }

    fn ok_response_with_custom_header(req: &Request<Incoming>) -> Response<Full<Bytes>> {
        let custom_value = req
            .headers()
            .get("x-custom-header")
            .map_or("not-found", |value| value.to_str().unwrap_or("missing"));
        Response::builder()
            .status(200)
            .header("content-type", "application/grpc")
            .header("grpc-status", "0")
            .header("x-echoed-custom", custom_value)
            .body(Full::new(Bytes::new()))
            .unwrap()
    }

    async fn request_once(pool: &BackendPool, socket_path: &str, i: usize) {
        let req = Request::builder()
            .method("POST")
            .uri(format!("/test.Service/Method{i}"))
            .header("content-type", "application/grpc")
            .body(Empty::<Bytes>::new())
            .unwrap();

        let result = proxy::forward(pool, req, socket_path).await;
        assert!(
            result.is_ok(),
            "Request {i} should succeed: {:?}",
            result.err()
        );
    }

    #[tokio::test]
    async fn proxy_to_backend_success() {
        // ARRANGE
        let backend = MockBackend::success()
            .await
            .expect("Failed to create mock backend");

        let req = Request::builder()
            .method("POST")
            .uri("/test.Service/Method")
            .header("content-type", "application/grpc")
            .body(Empty::<Bytes>::new())
            .unwrap();

        let socket_path = backend.socket_path().to_str().unwrap();
        let pool = BackendPool::from_socket(socket_path);

        // ACT
        let result = proxy::forward(&pool, req, socket_path).await;

        // ASSERT
        assert!(result.is_ok(), "Proxy should succeed: {:?}", result.err());

        let response = result.unwrap();
        assert_eq!(response.status(), 200);

        let grpc_status = response.headers().get("grpc-status");
        assert!(grpc_status.is_some(), "Should have grpc-status header");
        assert_eq!(grpc_status.unwrap(), "0");

        backend.shutdown().await;
    }

    #[tokio::test]
    async fn proxy_to_backend_connection_error() {
        // ARRANGE
        let req = Request::builder()
            .method("POST")
            .uri("/test.Service/Method")
            .body(Empty::<Bytes>::new())
            .unwrap();

        let socket_path = "/nonexistent/path/to/socket.sock";
        let pool = BackendPool::from_socket(socket_path);

        // ACT
        let result = proxy::forward(&pool, req, socket_path).await;

        // ASSERT
        assert!(result.is_err(), "Should fail for non-existent socket");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("Failed to connect"),
            "Error should mention connection failure: {err}"
        );
    }

    #[tokio::test]
    async fn proxy_to_backend_with_body() {
        // ARRANGE
        let backend = MockBackend::success()
            .await
            .expect("Failed to create mock backend");

        let body_content = b"test request body";
        let req = Request::builder()
            .method("POST")
            .uri("/test.Service/Method")
            .header("content-type", "application/grpc")
            .body(Full::new(Bytes::from_static(body_content)))
            .unwrap();

        let socket_path = backend.socket_path().to_str().unwrap();
        let pool = BackendPool::from_socket(socket_path);

        // ACT
        let result = proxy::forward(&pool, req, socket_path).await;

        // ASSERT
        assert!(
            result.is_ok(),
            "Proxy with body should succeed: {:?}",
            result.err()
        );

        backend.shutdown().await;
    }

    #[tokio::test]
    async fn proxy_receives_response_body() {
        // ARRANGE
        let backend = MockBackend::start(|_req| ok_response_with_body(b"response data"))
            .await
            .expect("Failed to create mock backend");

        let req = Request::builder()
            .method("POST")
            .uri("/test.Service/Method")
            .body(Empty::<Bytes>::new())
            .unwrap();

        let socket_path = backend.socket_path().to_str().unwrap();
        let pool = BackendPool::from_socket(socket_path);

        // ACT
        let response = proxy::forward(&pool, req, socket_path)
            .await
            .expect("Proxy should succeed");

        // ASSERT
        let body_bytes = response
            .into_body()
            .collect()
            .await
            .expect("Should collect body")
            .to_bytes();

        assert_eq!(body_bytes.as_ref(), b"response data");

        backend.shutdown().await;
    }

    #[tokio::test]
    async fn proxy_preserves_uri_path() {
        // ARRANGE
        let backend = MockBackend::start(|req| ok_response_with_path(req.uri().path()))
            .await
            .expect("Failed to create mock backend");

        let req = Request::builder()
            .method("POST")
            .uri("/muak.vm.v1.VmService/CreateVm")
            .header("content-type", "application/grpc")
            .body(Empty::<Bytes>::new())
            .unwrap();

        let socket_path = backend.socket_path().to_str().unwrap();
        let pool = BackendPool::from_socket(socket_path);

        // ACT
        let response = proxy::forward(&pool, req, socket_path)
            .await
            .expect("Proxy should succeed");

        // ASSERT
        let received_path = response.headers().get("x-received-path");
        assert!(received_path.is_some(), "Should have received path header");
        assert_eq!(
            received_path.unwrap(),
            "/muak.vm.v1.VmService/CreateVm",
            "Path should be preserved"
        );

        backend.shutdown().await;
    }

    #[tokio::test]
    async fn proxy_preserves_headers() {
        // ARRANGE
        let backend = MockBackend::start(|req| ok_response_with_custom_header(&req))
            .await
            .expect("Failed to create mock backend");

        let req = Request::builder()
            .method("POST")
            .uri("/test.Service/Method")
            .header("content-type", "application/grpc")
            .header("x-custom-header", "test-value-123")
            .body(Empty::<Bytes>::new())
            .unwrap();

        let socket_path = backend.socket_path().to_str().unwrap();
        let pool = BackendPool::from_socket(socket_path);

        // ACT
        let response = proxy::forward(&pool, req, socket_path)
            .await
            .expect("Proxy should succeed");

        // ASSERT
        let echoed = response.headers().get("x-echoed-custom");
        assert!(echoed.is_some(), "Should have echoed header");
        assert_eq!(echoed.unwrap(), "test-value-123", "Header value preserved");

        backend.shutdown().await;
    }

    #[tokio::test]
    async fn proxy_multiple_requests() {
        // ARRANGE
        let backend = MockBackend::success()
            .await
            .expect("Failed to create mock backend");

        let socket_path = backend.socket_path().to_str().unwrap();
        let pool = BackendPool::from_socket(socket_path);

        // ACT & ASSERT
        request_once(&pool, socket_path, 0).await;
        request_once(&pool, socket_path, 1).await;
        request_once(&pool, socket_path, 2).await;

        backend.shutdown().await;
    }

    #[tokio::test]
    async fn proxy_error_includes_socket_path() {
        // ARRANGE
        let socket_path = "/tmp/specific_test_socket_path_12345.sock";
        let pool = BackendPool::from_socket(socket_path);
        let req = Request::builder()
            .method("POST")
            .uri("/test")
            .body(Empty::<Bytes>::new())
            .unwrap();

        // ACT
        let result = proxy::forward(&pool, req, socket_path).await;

        // ASSERT
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains(socket_path),
            "Error should contain socket path '{socket_path}': {err}"
        );
    }
}
