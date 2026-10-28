use super::*;
use crate::{CallContext, FetchRequest};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{timeout, Duration};

async fn read_headers(stream: &mut TcpStream) -> String {
    timeout(Duration::from_secs(3), async {
        let mut bytes = Vec::new();
        let mut byte = [0u8; 1];
        while !bytes.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).await.unwrap();
            bytes.push(byte[0]);
            assert!(bytes.len() < 16 * 1024);
        }
        String::from_utf8(bytes).unwrap()
    })
    .await
    .unwrap()
}

#[test]
fn proxy_schemes_validate_without_disclosing_credentials() {
    for scheme in ["http", "https", "socks5", "socks5h"] {
        let proxy = NetworkProxy::Proxy {
            url: format!("{scheme}://private-user:private-password@127.0.0.1:1080"),
        };
        proxy.validate().unwrap();
        http_client(Some(&proxy)).unwrap();
        let view = serde_json::to_string(&proxy.redacted()).unwrap();
        let debug = format!("{proxy:?}");
        assert!(proxy.redacted().authenticated);
        for secret in ["private-user", "private-password"] {
            assert!(!view.contains(secret));
            assert!(!debug.contains(secret));
        }
    }
    for url in [
        "ftp://proxy.test:80",
        "http://proxy.test/path",
        "http://proxy.test?password=private-password",
        "http://proxy.test#private-password",
        "proxy.test:80",
    ] {
        let proxy = NetworkProxy::Proxy { url: url.into() };
        let error = http_client(Some(&proxy)).unwrap_err().to_string();
        assert!(!error.contains("private-password"));
    }
}

#[test]
fn proxy_parameters_remain_optional_on_the_wire() {
    let context: CallContext = serde_json::from_value(json!({})).unwrap();
    assert!(context.network_proxy.is_none());
    let request: FetchRequest = serde_json::from_value(json!({
        "url":"http://upstream.invalid", "method":"GET", "networkProxy":{"mode":"direct"}
    }))
    .unwrap();
    assert_eq!(request.network_proxy, Some(NetworkProxy::Direct));
}

#[tokio::test]
async fn http_requests_use_the_configured_proxy_and_its_authentication() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let proxy = NetworkProxy::Proxy {
        url: format!("http://private-user:private-password@{address}"),
    };
    let server = tokio::spawn(async move {
        let (mut socket, _) = timeout(Duration::from_secs(3), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let headers = read_headers(&mut socket).await;
        assert!(headers.starts_with("GET http://upstream.invalid/resource HTTP/1.1\r\n"));
        assert!(headers
            .to_ascii_lowercase()
            .contains("proxy-authorization: basic chjpdmf0zs11c2vyonbyaxzhdgutcgfzc3dvcmq="));
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nproxied")
            .await
            .unwrap();
    });
    let body = http_client(Some(&proxy))
        .unwrap()
        .get("http://upstream.invalid/resource")
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(body, "proxied");
    server.await.unwrap();
}

#[tokio::test]
async fn an_unavailable_proxy_never_falls_back_to_a_direct_connection() {
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let unavailable = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = NetworkProxy::Proxy {
        url: format!("http://{}", unavailable.local_addr().unwrap()),
    };
    drop(unavailable);
    let result = http_client_builder(Some(&proxy))
        .unwrap()
        .resolve("origin.nora-proxy.test", origin.local_addr().unwrap())
        .build()
        .unwrap()
        .get(format!(
            "http://origin.nora-proxy.test:{}/",
            origin.local_addr().unwrap().port()
        ))
        .timeout(Duration::from_secs(3))
        .send()
        .await;
    assert!(result.is_err());
    assert!(timeout(Duration::from_millis(100), origin.accept())
        .await
        .is_err());
}

#[tokio::test]
async fn socks5_and_socks5h_select_local_and_remote_name_resolution() {
    for (scheme, target, expected_address_type) in [
        ("socks5", "192.0.2.1", 1u8),
        ("socks5h", "upstream.invalid", 3u8),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy = NetworkProxy::Proxy {
            url: format!("{scheme}://{}", listener.local_addr().unwrap()),
        };
        let server = tokio::spawn(async move {
            let (mut socket, _) = timeout(Duration::from_secs(3), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut greeting = [0u8; 2];
            socket.read_exact(&mut greeting).await.unwrap();
            assert_eq!(greeting[0], 5);
            let mut methods = vec![0u8; greeting[1] as usize];
            socket.read_exact(&mut methods).await.unwrap();
            assert!(methods.contains(&0));
            socket.write_all(&[5, 0]).await.unwrap();
            let mut head = [0u8; 4];
            socket.read_exact(&mut head).await.unwrap();
            assert_eq!(head, [5, 1, 0, expected_address_type]);
            let length = if head[3] == 3 {
                let mut length = [0u8; 1];
                socket.read_exact(&mut length).await.unwrap();
                length[0] as usize
            } else {
                4
            };
            let mut address = vec![0u8; length];
            socket.read_exact(&mut address).await.unwrap();
            if head[3] == 3 {
                assert_eq!(address, b"upstream.invalid");
            }
            let mut port = [0u8; 2];
            socket.read_exact(&mut port).await.unwrap();
            assert_eq!(u16::from_be_bytes(port), 80);
            socket
                .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
                .await
                .unwrap();
            assert!(read_headers(&mut socket)
                .await
                .starts_with("GET /resource HTTP/1.1\r\n"));
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nsocks",
                )
                .await
                .unwrap();
        });
        let body = http_client(Some(&proxy))
            .unwrap()
            .get(format!("http://{target}/resource"))
            .timeout(Duration::from_secs(3))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "socks");
        server.await.unwrap();
    }
}
