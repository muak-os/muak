//! Integration tests for the proxy functionality.

mod fixtures;

use apid::proxy::{self, BackendPool};
use fixtures::mock_backend::MockBackend;
use http_body_util::{BodyExt, Empty, Full};
use hyper::Request;
use hyper::body::Bytes;

#[tokio::test]
async fn test_proxy_to_backend_success() {
    let backend = MockBackend::success()
        .await
        .expect("Failed to create mock backend");

    let req = Request::builder()
        .method("POST")
        .uri("/test.Service/Method")
        .header("content-type", "application/grpc")
        .body(Empty::<Bytes>::new())
        .unwrap();

    let socket_path = backend.socket_path.to_str().unwrap();
    let pool = BackendPool::from_socket(socket_path);
    let result = proxy::proxy_to_backend(&pool, req, socket_path).await;

    assert!(result.is_ok(), "Proxy should succeed: {:?}", result.err());

    let response = result.unwrap();
    assert_eq!(response.status(), 200);

    let grpc_status = response.headers().get("grpc-status");
    assert!(grpc_status.is_some(), "Should have grpc-status header");
    assert_eq!(grpc_status.unwrap(), "0");

    backend.shutdown().await;
}

#[tokio::test]
async fn test_proxy_to_backend_connection_error() {
    let req = Request::builder()
        .method("POST")
        .uri("/test.Service/Method")
        .body(Empty::<Bytes>::new())
        .unwrap();

    let socket_path = "/nonexistent/path/to/socket.sock";
    let pool = BackendPool::from_socket(socket_path);
    let result = proxy::proxy_to_backend(&pool, req, socket_path).await;

    assert!(result.is_err(), "Should fail for non-existent socket");
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("Failed to connect"),
        "Error should mention connection failure: {}",
        err
    );
}

#[tokio::test]
async fn test_proxy_to_backend_with_body() {
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

    let socket_path = backend.socket_path.to_str().unwrap();
    let pool = BackendPool::from_socket(socket_path);
    let result = proxy::proxy_to_backend(&pool, req, socket_path).await;

    assert!(
        result.is_ok(),
        "Proxy with body should succeed: {:?}",
        result.err()
    );

    backend.shutdown().await;
}

#[tokio::test]
async fn test_proxy_receives_response_body() {
    let backend = MockBackend::start(|_req| {
        hyper::Response::builder()
            .status(200)
            .header("content-type", "application/grpc")
            .header("grpc-status", "0")
            .body(http_body_util::Full::new(Bytes::from_static(
                b"response data",
            )))
            .unwrap()
    })
    .await
    .expect("Failed to create mock backend");

    let req = Request::builder()
        .method("POST")
        .uri("/test.Service/Method")
        .body(Empty::<Bytes>::new())
        .unwrap();

    let socket_path = backend.socket_path.to_str().unwrap();
    let pool = BackendPool::from_socket(socket_path);
    let response = proxy::proxy_to_backend(&pool, req, socket_path)
        .await
        .expect("Proxy should succeed");

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
async fn test_proxy_preserves_uri_path() {
    let backend = MockBackend::start(|req| {
        hyper::Response::builder()
            .status(200)
            .header("content-type", "application/grpc")
            .header("grpc-status", "0")
            .header("x-received-path", req.uri().path())
            .body(http_body_util::Full::new(Bytes::new()))
            .unwrap()
    })
    .await
    .expect("Failed to create mock backend");

    let req = Request::builder()
        .method("POST")
        .uri("/muak.vm.v1.VmService/CreateVm")
        .header("content-type", "application/grpc")
        .body(Empty::<Bytes>::new())
        .unwrap();

    let socket_path = backend.socket_path.to_str().unwrap();
    let pool = BackendPool::from_socket(socket_path);
    let response = proxy::proxy_to_backend(&pool, req, socket_path)
        .await
        .expect("Proxy should succeed");

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
async fn test_proxy_preserves_headers() {
    let backend = MockBackend::start(|req| {
        let custom_value = req
            .headers()
            .get("x-custom-header")
            .map(|v| v.to_str().unwrap_or("missing"))
            .unwrap_or("not-found");

        hyper::Response::builder()
            .status(200)
            .header("content-type", "application/grpc")
            .header("grpc-status", "0")
            .header("x-echoed-custom", custom_value)
            .body(http_body_util::Full::new(Bytes::new()))
            .unwrap()
    })
    .await
    .expect("Failed to create mock backend");

    let req = Request::builder()
        .method("POST")
        .uri("/test.Service/Method")
        .header("content-type", "application/grpc")
        .header("x-custom-header", "test-value-123")
        .body(Empty::<Bytes>::new())
        .unwrap();

    let socket_path = backend.socket_path.to_str().unwrap();
    let pool = BackendPool::from_socket(socket_path);
    let response = proxy::proxy_to_backend(&pool, req, socket_path)
        .await
        .expect("Proxy should succeed");

    let echoed = response.headers().get("x-echoed-custom");
    assert!(echoed.is_some(), "Should have echoed header");
    assert_eq!(echoed.unwrap(), "test-value-123", "Header value preserved");

    backend.shutdown().await;
}

#[tokio::test]
async fn test_proxy_multiple_requests() {
    let backend = MockBackend::success()
        .await
        .expect("Failed to create mock backend");

    let socket_path = backend.socket_path.to_str().unwrap();
    let pool = BackendPool::from_socket(socket_path);

    for i in 0..3 {
        let req = Request::builder()
            .method("POST")
            .uri(format!("/test.Service/Method{}", i))
            .header("content-type", "application/grpc")
            .body(Empty::<Bytes>::new())
            .unwrap();

        let result = proxy::proxy_to_backend(&pool, req, socket_path).await;
        assert!(
            result.is_ok(),
            "Request {} should succeed: {:?}",
            i,
            result.err()
        );
    }

    backend.shutdown().await;
}

#[tokio::test]
async fn test_proxy_error_includes_socket_path() {
    let socket_path = "/tmp/specific_test_socket_path_12345.sock";
    let pool = BackendPool::from_socket(socket_path);
    let req = Request::builder()
        .method("POST")
        .uri("/test")
        .body(Empty::<Bytes>::new())
        .unwrap();

    let result = proxy::proxy_to_backend(&pool, req, socket_path).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains(socket_path),
        "Error should contain socket path '{}': {}",
        socket_path,
        err
    );
}
