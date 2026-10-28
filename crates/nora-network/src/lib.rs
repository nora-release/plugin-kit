pub mod tls;

/// Keep Rustls protocol support while evaluating macOS certificate trust using
/// the system verifier. Other platforms retain reqwest's existing root policy.
pub fn http_client_builder() -> Result<reqwest::ClientBuilder, rustls::Error> {
    let builder = reqwest::Client::builder();
    #[cfg(feature = "ipv4-fallback")]
    let builder = builder.ipv4_fallback(true);
    #[cfg(feature = "http2")]
    let builder = builder
        .http2_keep_alive_interval(std::time::Duration::from_secs(20))
        .http2_keep_alive_timeout(std::time::Duration::from_secs(10))
        .http2_keep_alive_while_idle(true);
    #[cfg(target_os = "macos")]
    let builder = builder.use_preconfigured_tls(tls::http_client_config(std::iter::empty())?);
    Ok(builder)
}

#[cfg(all(test, feature = "http2"))]
#[path = "../test/http2_health.test.rs"]
mod http2_health_tests;

#[cfg(test)]
#[allow(dead_code)]
#[path = "../vendor/reqwest/src/connect/ipv4_preference.rs"]
mod ipv4_preference_tests;
