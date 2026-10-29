use super::{NetworkProxy, PluginError, PluginResult};
use reqwest::Url;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct ProxyDecision {
    pub proxy: Option<NetworkProxy>,
    pub source: &'static str,
    pub bypassed: bool,
}

pub fn proxy_for_url(
    explicit: Option<&NetworkProxy>,
    bypass_lan: bool,
    url: &str,
) -> PluginResult<ProxyDecision> {
    let target =
        Url::parse(url).map_err(|_| PluginError::invalid("The network target is invalid."))?;
    let environment = std::env::vars()
        .filter(|(key, _)| {
            matches!(
                key.as_str(),
                "HTTP_PROXY"
                    | "http_proxy"
                    | "HTTPS_PROXY"
                    | "https_proxy"
                    | "ALL_PROXY"
                    | "all_proxy"
                    | "NO_PROXY"
                    | "no_proxy"
                    | "REQUEST_METHOD"
            )
        })
        .collect::<BTreeMap<_, _>>();
    let host = target.host_str().unwrap_or_default();
    // Local services and OAuth callbacks must remain reachable with any setting.
    if super::lan::loopback(host) {
        return Ok(ProxyDecision {
            proxy: None,
            source: "local",
            bypassed: true,
        });
    }
    let mut selected = select(
        explicit,
        &target,
        &environment,
        &super::system::SystemProxy::default(),
    )?;
    if explicit.is_none() && selected.source == "none" {
        selected = select(None, &target, &environment, &super::system::current())?;
    }
    if selected.proxy.is_some() && bypass_lan && super::lan::on_lan(host) {
        return Ok(ProxyDecision {
            proxy: None,
            source: selected.source,
            bypassed: true,
        });
    }
    Ok(selected)
}

pub(super) fn select(
    explicit: Option<&NetworkProxy>,
    target: &Url,
    environment: &BTreeMap<String, String>,
    system: &super::system::SystemProxy,
) -> PluginResult<ProxyDecision> {
    let direct = |source, bypassed| ProxyDecision {
        proxy: None,
        source,
        bypassed,
    };
    let host = target.host_str().unwrap_or_default();
    if super::lan::loopback(host) {
        return Ok(direct("local", true));
    }
    if let Some(explicit) = explicit {
        explicit.validate()?;
        return Ok(match explicit {
            NetworkProxy::Direct => direct("off", false),
            proxy => ProxyDecision {
                proxy: Some(proxy.clone()),
                source: "settings",
                bypassed: false,
            },
        });
    }
    let env = |keys: &[&str]| {
        keys.iter().find_map(|key| {
            environment
                .get(*key)
                .filter(|value| !value.trim().is_empty())
                .map(|value| value.trim())
        })
    };
    let no_proxy = env(&["NO_PROXY", "no_proxy"]).unwrap_or_default();
    if bypass(
        host,
        &no_proxy.split(',').map(str::to_owned).collect::<Vec<_>>(),
        true,
    ) {
        return Ok(direct("environment", true));
    }
    let scheme = if target.scheme() == "http" {
        &["HTTP_PROXY", "http_proxy"][..]
    } else {
        &["HTTPS_PROXY", "https_proxy"][..]
    };
    let proxy = env(scheme).or_else(|| env(&["ALL_PROXY", "all_proxy"]));
    if let Some(raw) = proxy {
        if target.scheme() == "http"
            && environment.contains_key("REQUEST_METHOD")
            && environment.contains_key("HTTP_PROXY")
        {
            return Err(PluginError::invalid(
                "HTTP_PROXY is unavailable in a CGI environment.",
            ));
        }
        return Ok(ProxyDecision {
            proxy: Some(parse(raw)?),
            source: "environment",
            bypassed: false,
        });
    }
    if bypass(host, &system.bypass, false) {
        return Ok(direct("system", true));
    }
    let raw = if target.scheme() == "http" {
        system.http.as_ref().or(system.socks.as_ref())
    } else {
        system
            .https
            .as_ref()
            .or(system.http.as_ref())
            .or(system.socks.as_ref())
    };
    match raw {
        Some(raw) => Ok(ProxyDecision {
            proxy: Some(parse(raw)?),
            source: "system",
            bypassed: false,
        }),
        None => Ok(direct("none", false)),
    }
}

fn parse(raw: &str) -> PluginResult<NetworkProxy> {
    let url = if raw.contains("://") {
        raw.to_owned()
    } else {
        format!("http://{raw}")
    };
    let proxy = NetworkProxy::Proxy { url };
    proxy.validate()?;
    Ok(proxy)
}

pub(super) fn bypass(host: &str, entries: &[String], domains: bool) -> bool {
    let host = host
        .trim_end_matches('.')
        .trim_matches(['[', ']'])
        .to_ascii_lowercase();
    for entry in entries {
        let raw = entry.trim().to_ascii_lowercase();
        if raw == "*" {
            return true;
        }
        if raw == "<local>" && !host.contains('.') && !host.contains(':') {
            return true;
        }
        if let Some((address, bits)) = raw.split_once('/') {
            if cidr(&host, address, bits) {
                return true;
            }
            continue;
        }
        let raw = if raw.starts_with('[') {
            raw.find(']')
                .map(|end| raw[1..end].to_owned())
                .unwrap_or(raw)
        } else if raw.matches(':').count() == 1 {
            raw.split_once(':')
                .map(|(name, _)| name.to_owned())
                .unwrap_or(raw)
        } else {
            raw
        };
        let suffix = raw.starts_with("*.") || raw.starts_with('.');
        let entry = raw
            .trim_start_matches('*')
            .trim_start_matches('.')
            .trim_end_matches('.');
        if !entry.is_empty()
            && (host == entry || (domains || suffix) && host.ends_with(&format!(".{entry}")))
        {
            return true;
        }
    }
    false
}

fn cidr(host: &str, address: &str, bits: &str) -> bool {
    use std::net::IpAddr;
    let (Ok(host), Ok(address), Ok(bits)) = (
        host.parse::<IpAddr>(),
        address.parse::<IpAddr>(),
        bits.parse::<u32>(),
    ) else {
        return false;
    };
    match (host, address) {
        (IpAddr::V4(host), IpAddr::V4(address)) if bits <= 32 => {
            let mask = u32::MAX.checked_shl(32 - bits).unwrap_or(0);
            u32::from(host) & mask == u32::from(address) & mask
        }
        (IpAddr::V6(host), IpAddr::V6(address)) if bits <= 128 => {
            let mask = u128::MAX.checked_shl(128 - bits).unwrap_or(0);
            u128::from(host) & mask == u128::from(address) & mask
        }
        _ => false,
    }
}
