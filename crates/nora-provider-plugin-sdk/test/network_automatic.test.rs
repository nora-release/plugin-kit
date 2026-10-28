use super::*;
use automatic::{bypass, select};
use reqwest::Url;
use std::collections::BTreeMap;

fn system() -> system::SystemProxy {
    system::parse("<dictionary> {\nHTTPSEnable : 1\nHTTPSProxy : 127.0.0.1\nHTTPSPort : 7890\nHTTPEnable : 1\nHTTPProxy : 127.0.0.2\nHTTPPort : 7891\nSOCKSEnable : 1\nSOCKSProxy : 127.0.0.3\nSOCKSPort : 1080\nExceptionsList : <array> {\n0 : *.private.example\n1 : <local>\n}\n__SCOPED__ : <dictionary> {\nHTTPSEnable : 1\nHTTPSProxy : evil.invalid\nHTTPSPort : 90\n}\n}")
}

#[test]
fn automatic_proxy_precedence_and_protocols_match_magpie() {
    let url = Url::parse("https://api.example/v1").unwrap();
    let env = BTreeMap::from([("HTTPS_PROXY".into(), "http://env.invalid:9000".into())]);
    assert_eq!(
        select(None, &url, &env, &system()).unwrap().source,
        "environment"
    );
    assert_eq!(
        select(Some(&NetworkProxy::Direct), &url, &env, &system())
            .unwrap()
            .source,
        "off"
    );
    let own = NetworkProxy::Proxy {
        url: "socks5h://own.invalid:1080".into(),
    };
    assert_eq!(
        select(Some(&own), &url, &env, &system()).unwrap().proxy,
        Some(own)
    );
    let empty = BTreeMap::new();
    let https = select(None, &url, &empty, &system()).unwrap();
    assert_eq!(https.source, "system");
    assert_eq!(
        https.proxy.unwrap().redacted().url.as_deref(),
        Some("http://127.0.0.1:7890/")
    );
    let http = select(
        None,
        &Url::parse("http://api.example").unwrap(),
        &empty,
        &system(),
    )
    .unwrap();
    assert_eq!(
        http.proxy.unwrap().redacted().url.as_deref(),
        Some("http://127.0.0.2:7891/")
    );
    assert!(
        select(
            None,
            &Url::parse("https://nas.private.example").unwrap(),
            &empty,
            &system()
        )
        .unwrap()
        .bypassed
    );
    assert!(
        select(None, &Url::parse("https://nas").unwrap(), &empty, &system())
            .unwrap()
            .bypassed
    );
}

#[test]
fn bypasses_cover_domains_ports_ipv6_and_cidr_without_suffix_confusion() {
    for (host, list, domains, want) in [
        ("api.example.com", "example.com:443", true, true),
        ("notexample.com", "example.com", true, false),
        ("api.example.com", "example.com", false, false),
        ("api.example.com", "*.example.com", false, true),
        ("2001:db8::1", "[2001:db8::1]:443", true, true),
        ("10.10.1.2", "10.0.0.0/8", true, true),
        ("100.1.1.2", "10.0.0.0/8", true, false),
        ("fe80::1", "fe80::/10", true, true),
        ("anything.example", "*", true, true),
    ] {
        assert_eq!(bypass(host, &[list.into()], domains), want, "{host} {list}");
    }
    for host in ["localhost", "foo.localhost", "127.2.3.4", "[::1]"] {
        let url = Url::parse(&format!("http://{host}")).unwrap();
        assert!(select(
            Some(&NetworkProxy::Proxy {
                url: "http://proxy.invalid:8080".into()
            }),
            &url,
            &BTreeMap::new(),
            &system()
        )
        .unwrap()
        .proxy
        .is_none());
    }
    for ip in [
        "192.168.1.1",
        "10.0.0.1",
        "172.16.0.1",
        "100.64.0.1",
        "169.254.1.1",
        "fc00::1",
        "fe80::1",
        "::ffff:192.168.1.1",
    ] {
        assert!(lan::lan_ip(ip.parse().unwrap()));
    }
    for ip in ["8.8.8.8", "100.128.0.1", "2001:4860::1"] {
        assert!(!lan::lan_ip(ip.parse().unwrap()));
    }
}

#[test]
fn no_proxy_and_bad_proxy_never_fall_through_to_system() {
    let url = Url::parse("https://api.example").unwrap();
    let env = BTreeMap::from([(
        "HTTPS_PROXY".into(),
        "ftp://secret:password@bad.invalid".into(),
    )]);
    assert!(select(None, &url, &env, &system()).is_err());
    let env = BTreeMap::from([
        ("NO_PROXY".into(), "api.example".into()),
        ("ALL_PROXY".into(), "http://env.invalid:90".into()),
    ]);
    assert!(select(None, &url, &env, &system()).unwrap().proxy.is_none());
    let env = BTreeMap::from([("ALL_PROXY".into(), "socks5h://fallback.invalid:1080".into())]);
    assert_eq!(
        select(None, &url, &env, &system()).unwrap().source,
        "environment"
    );
}

#[tokio::test]
async fn loopback_remains_direct_even_when_the_explicit_proxy_is_unavailable() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/", origin.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut socket, _) = origin.accept().await.unwrap();
        let mut bytes = [0; 1024];
        let _ = socket.read(&mut bytes).await.unwrap();
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .unwrap();
    });
    let proxy = NetworkProxy::Proxy {
        url: "http://127.0.0.1:1".into(),
    };
    let result = http_client(Some(&proxy))
        .unwrap()
        .get(endpoint)
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
        .unwrap();
    assert_eq!(result.text().await.unwrap(), "ok");
    task.await.unwrap();
}

#[test]
fn system_parser_does_not_take_disabled_or_scoped_proxy() {
    let parsed=system::parse("<dictionary> {\nHTTPEnable : 0\nHTTPProxy : bad.invalid\nHTTPPort : 90\n__SCOPED__ : <dictionary> {\nHTTPSEnable : 1\nHTTPSProxy : bad.invalid\nHTTPSPort : 91\n}\n}");
    assert!(parsed.http.is_none());
    assert!(parsed.https.is_none());
}

#[test]
fn a_system_proxy_started_after_nora_is_discovered_without_restarting_clients() {
    let now = std::time::Instant::now();
    let mut cache = None;
    assert!(
        system::cached(&mut cache, now, system::SystemProxy::default)
            .https
            .is_none()
    );
    assert!(system::cached(
        &mut cache,
        now + std::time::Duration::from_secs(1),
        || panic!("empty result is still fresh")
    )
    .https
    .is_none());
    assert!(
        system::cached(&mut cache, now + std::time::Duration::from_secs(2), system)
            .https
            .is_some()
    );
    assert!(system::cached(
        &mut cache,
        now + std::time::Duration::from_secs(16),
        || panic!("configured proxy is still fresh")
    )
    .https
    .is_some());
    assert!(system::cached(
        &mut cache,
        now + std::time::Duration::from_secs(17),
        system::SystemProxy::default
    )
    .https
    .is_none());
}

#[test]
fn global_proxies_bypass_lan_while_a_provider_own_proxy_remains_authoritative() {
    let proxy = NetworkProxy::Proxy {
        url: "http://proxy.invalid:8080".into(),
    };
    assert!(proxy_for_url(Some(&proxy), true, "http://10.1.2.3/")
        .unwrap()
        .proxy
        .is_none());
    assert!(proxy_for_url(Some(&proxy), false, "http://10.1.2.3/")
        .unwrap()
        .proxy
        .is_some());
    assert!(proxy_for_url(Some(&proxy), false, "http://127.0.0.1/")
        .unwrap()
        .proxy
        .is_none());
}
