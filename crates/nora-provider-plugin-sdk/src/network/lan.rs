use std::collections::BTreeMap;
use std::net::{IpAddr, ToSocketAddrs};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub(super) fn loopback(host: &str) -> bool {
    let host = host
        .trim_matches(['[', ']'])
        .trim_end_matches('.')
        .to_ascii_lowercase();
    host == "localhost"
        || host.ends_with(".localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

pub(super) fn lan_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || u32::from(ip) & 0xffc00000 == 0x64400000
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.segments()[0] & 0xfe00 == 0xfc00
                || ip.segments()[0] & 0xffc0 == 0xfe80
                || ip.to_ipv4_mapped().is_some_and(|ip| lan_ip(IpAddr::V4(ip)))
        }
    }
}

pub(super) fn on_lan(host: &str) -> bool {
    let host = host
        .trim_matches(['[', ']'])
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if loopback(&host) || host.ends_with(".local") {
        return true;
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return lan_ip(ip);
    }
    type Cache = BTreeMap<String, (Instant, bool)>;
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    static ACTIVE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let cache = CACHE.get_or_init(Mutex::default);
    if let Ok(cache) = cache.lock() {
        if let Some((at, lan)) = cache.get(&host) {
            if at.elapsed() < Duration::from_secs(60) {
                return *lan;
            }
        }
    }
    use std::sync::atomic::Ordering;
    if ACTIVE.fetch_add(1, Ordering::AcqRel) >= 8 {
        ACTIVE.fetch_sub(1, Ordering::AcqRel);
        return false;
    }
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let Ok(_) = std::thread::Builder::new()
        .name("nora-proxy-dns".into())
        .spawn(move || {
            let addresses = (host.as_str(), 0)
                .to_socket_addrs()
                .map(|values| values.map(|value| value.ip()).collect::<Vec<_>>());
            let lan = addresses
                .is_ok_and(|addresses| !addresses.is_empty() && addresses.into_iter().all(lan_ip));
            if let Ok(mut cache) = cache.lock() {
                if cache.len() >= 256 {
                    cache.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(60));
                }
                if cache.len() >= 256 {
                    cache.clear();
                }
                cache.insert(host, (Instant::now(), lan));
            }
            ACTIVE.fetch_sub(1, Ordering::AcqRel);
            let _ = sender.send(lan);
        })
    else {
        ACTIVE.fetch_sub(1, Ordering::AcqRel);
        return false;
    };
    receiver
        .recv_timeout(Duration::from_millis(400))
        .unwrap_or(false)
}
