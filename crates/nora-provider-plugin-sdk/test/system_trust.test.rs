use super::*;
#[path = "system_tls_support.rs"]
mod tls;
#[tokio::test]
async fn sdk_checks_certificates_for_both_target_and_https_proxy() {
    tls::check_http_trust(|proxy, roots| {
        let policy = proxy
            .map(|url| NetworkProxy::Proxy { url: url.into() })
            .unwrap_or(NetworkProxy::Direct);
        http_client_builder(Some(&policy))
            .unwrap()
            .use_preconfigured_tls(nora_network::tls::http_client_config(roots).unwrap())
    })
    .await;
}

#[tokio::test]
async fn actual_sdk_default_client_rejects_an_untrusted_certificate() {
    let ca = tls::Authority::new();
    let target = tls::Server::start(&ca, &["localhost"], false, b"untrusted").await;
    assert!(http_client(Some(&NetworkProxy::Direct))
        .unwrap()
        .get(target.url("localhost"))
        .send()
        .await
        .is_err());
}
