use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Default)]
pub(super) struct SystemProxy {
    pub http: Option<String>,
    pub https: Option<String>,
    pub socks: Option<String>,
    pub bypass: Vec<String>,
}

pub(super) fn current() -> SystemProxy {
    static CACHE: OnceLock<Mutex<Option<(Instant, SystemProxy)>>> = OnceLock::new();
    let Ok(mut cache) = CACHE.get_or_init(Mutex::default).lock() else {
        return SystemProxy::default();
    };
    cached(&mut cache, Instant::now(), read)
}

pub(super) fn cached(
    cache: &mut Option<(Instant, SystemProxy)>,
    now: Instant,
    reader: impl FnOnce() -> SystemProxy,
) -> SystemProxy {
    if let Some((at, proxy)) = cache.as_ref() {
        let keep = if proxy.http.is_some() || proxy.https.is_some() || proxy.socks.is_some() {
            15
        } else {
            2
        };
        if now.saturating_duration_since(*at) < Duration::from_secs(keep) {
            return proxy.clone();
        }
    }
    let proxy = reader();
    *cache = Some((now, proxy.clone()));
    proxy
}

#[cfg(target_os = "macos")]
fn read() -> SystemProxy {
    use std::io::Read;
    use std::process::{Command, Stdio};
    let Ok(mut child) = Command::new("/usr/sbin/scutil")
        .arg("--proxy")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return SystemProxy::default();
    };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return SystemProxy::default(),
            Ok(None) if started.elapsed() < Duration::from_secs(1) => {
                std::thread::sleep(Duration::from_millis(5))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return SystemProxy::default();
            }
        }
    }
    let mut output = String::new();
    if child
        .stdout
        .take()
        .is_none_or(|stdout| stdout.take(32 * 1024).read_to_string(&mut output).is_err())
    {
        return SystemProxy::default();
    }
    parse(&output)
}

#[cfg(not(target_os = "macos"))]
fn read() -> SystemProxy {
    SystemProxy::default()
}

pub(super) fn parse(output: &str) -> SystemProxy {
    let mut values = std::collections::BTreeMap::new();
    let mut bypass = Vec::new();
    let mut exceptions = false;
    let mut depth = 0i32;
    for line in output.lines().take(512) {
        let line = line.trim();
        if line.ends_with('{') {
            depth += 1;
            if depth == 2 && line.starts_with("ExceptionsList") {
                exceptions = true;
            }
            continue;
        }
        if line == "}" {
            if depth == 2 {
                exceptions = false;
            }
            depth -= 1;
            continue;
        }
        let Some((key, value)) = line.split_once(" : ") else {
            continue;
        };
        if depth == 1 {
            values.insert(key.trim(), value.trim());
        }
        if depth == 2 && exceptions {
            bypass.push(value.trim().to_owned());
        }
    }
    let endpoint = |name: &str, scheme: &str| {
        if values.get(format!("{name}Enable").as_str()).copied() != Some("1") {
            return None;
        }
        let host = values.get(format!("{name}Proxy").as_str())?;
        let port = values
            .get(format!("{name}Port").as_str())?
            .parse::<u16>()
            .ok()?;
        if port == 0 {
            return None;
        }
        let host = if host.contains(':') && !host.starts_with('[') {
            format!("[{host}]")
        } else {
            host.to_string()
        };
        let url = format!("{scheme}://{host}:{port}");
        let proxy = super::NetworkProxy::Proxy { url: url.clone() };
        proxy.validate().ok().map(|_| url)
    };
    SystemProxy {
        http: endpoint("HTTP", "http"),
        https: endpoint("HTTPS", "http"),
        socks: endpoint("SOCKS", "socks5"),
        bypass,
    }
}
