use crate::{PluginError, PluginResult};
use serde::{Deserialize, Serialize};
use std::fmt;
mod automatic;
mod lan;
mod system;
pub use automatic::{proxy_for_url, ProxyDecision};

tokio::task_local! { static REQUEST_PROXY: (bool, bool); }

pub(crate) async fn scope<T>(
    automatic: bool,
    bypass_lan: bool,
    future: impl std::future::Future<Output = T>,
) -> T {
    REQUEST_PROXY.scope((automatic, bypass_lan), future).await
}

#[derive(Clone, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(tag = "mode", rename_all = "camelCase", deny_unknown_fields)]
pub enum NetworkProxy {
    #[default]
    Direct,
    Proxy {
        url: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NetworkProxyView {
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub authenticated: bool,
}

impl NetworkProxy {
    pub fn validate(&self) -> PluginResult<()> {
        self.proxy_url().map(|_| ())
    }

    fn proxy_url(&self) -> PluginResult<Option<reqwest::Url>> {
        let Self::Proxy { url } = self else {
            return Ok(None);
        };
        let invalid = || {
            PluginError::invalid("Use an HTTP, HTTPS, SOCKS5, or SOCKS5h proxy URL without a path, query, or fragment.")
        };
        if url.len() > 8192 || url.trim() != url || url.contains(['\r', '\n']) {
            return Err(invalid());
        }
        let parsed = reqwest::Url::parse(url).map_err(|_| invalid())?;
        if !matches!(parsed.scheme(), "http" | "https" | "socks5" | "socks5h")
            || parsed.host_str().is_none()
            || !matches!(parsed.path(), "" | "/")
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err(invalid());
        }
        Ok(Some(parsed))
    }

    pub fn redacted(&self) -> NetworkProxyView {
        let Ok(Some(mut url)) = self.proxy_url() else {
            return NetworkProxyView {
                mode: "direct".into(),
                url: None,
                authenticated: false,
            };
        };
        let authenticated = !url.username().is_empty() || url.password().is_some();
        let _ = url.set_username("");
        let _ = url.set_password(None);
        NetworkProxyView {
            mode: "proxy".into(),
            url: Some(url.to_string()),
            authenticated,
        }
    }
}

impl fmt::Debug for NetworkProxy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.redacted().fmt(formatter)
    }
}

/// An explicit setting wins; automatic requests follow environment and macOS
/// settings at connection time, including changes after Nora was started.
pub fn http_client_builder(proxy: Option<&NetworkProxy>) -> PluginResult<reqwest::ClientBuilder> {
    let policy = REQUEST_PROXY
        .try_with(|value| *value)
        .unwrap_or((proxy.is_none(), proxy.is_none()));
    builder(proxy, policy)
}

pub fn global_http_client_builder(
    proxy: Option<&NetworkProxy>,
) -> PluginResult<reqwest::ClientBuilder> {
    builder(proxy, (proxy.is_none(), true))
}

fn builder(
    proxy: Option<&NetworkProxy>,
    (automatic, bypass_lan): (bool, bool),
) -> PluginResult<reqwest::ClientBuilder> {
    let builder = nora_network::http_client_builder()
        .map_err(|_| PluginError::transport("The provider TLS client could not be initialized."))?
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none());
    if let Some(proxy) = proxy {
        proxy.validate()?;
    }
    let explicit = if automatic { None } else { proxy.cloned() };
    if explicit == Some(NetworkProxy::Direct) {
        return Ok(builder);
    }
    let selected = reqwest::Proxy::custom(move |url| {
        match proxy_for_url(explicit.as_ref(), bypass_lan, url.as_str()) {
            Ok(ProxyDecision {
                proxy: Some(NetworkProxy::Proxy { url }),
                ..
            }) => Some(url),
            Ok(_) => None,
            // A broken proxy setting must not silently leak traffic directly.
            Err(_) => Some("http://127.0.0.1:0".to_owned()),
        }
    });
    Ok(builder.proxy(selected))
}

pub fn http_client(proxy: Option<&NetworkProxy>) -> PluginResult<reqwest::Client> {
    http_client_builder(proxy)?
        .build()
        .map_err(|_| PluginError::transport("The provider HTTP client could not be initialized."))
}

/// Send an HTTP request only after Nora has admitted this exact attempt.
pub async fn send_http(request: reqwest::RequestBuilder) -> PluginResult<reqwest::Response> {
    crate::before_http_request().await?;
    request
        .send()
        .await
        .map_err(|_| PluginError::transport("The provider HTTP request could not be completed."))
}

#[cfg(test)]
#[path = "../test/network.test.rs"]
mod tests;

#[cfg(test)]
#[path = "../test/network_automatic.test.rs"]
mod automatic_tests;

#[cfg(all(test, target_os = "macos"))]
#[path = "../test/system_trust.test.rs"]
mod system_trust_tests;

#[cfg(all(test, feature = "ipv4-fallback"))]
#[path = "../test/ipv4_fallback.test.rs"]
mod ipv4_fallback_tests;

#[cfg(all(test, feature = "ipv4-fallback"))]
#[path = "../test/ipv4_boundaries.test.rs"]
mod ipv4_boundary_tests;
