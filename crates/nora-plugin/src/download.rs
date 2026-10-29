use nora_plugin_manifest::{canonical, verify_archive, Error, Result, MAXIMUM_ARCHIVE_BYTES};
use serde_json::Value;
use std::collections::HashSet;
use std::time::Duration;
use url::Url;

const RELEASE_HOSTS: [&str; 3] = [
    "github.com",
    "release-assets.githubusercontent.com",
    "objects.githubusercontent.com",
];
const MAXIMUM_REDIRECTS: usize = 5;
const MAXIMUM_CHANGED_PLUGINS: usize = 100;
const MAXIMUM_SUBMISSION_BYTES: usize = 1024 * 1024 * 1024;
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(180);

/// One HTTP GET that never follows redirects.
pub struct Reply {
    pub status: u16,
    pub location: Option<String>,
    pub body: Vec<u8>,
}

pub trait Fetch {
    /// Reads at most `limit` body bytes, failing on a longer body.
    fn fetch(&self, url: &Url, limit: usize) -> Result<Reply>;
}

pub struct Network {
    runtime: tokio::runtime::Runtime,
    client: reqwest::Client,
}

impl Network {
    pub fn new() -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| Error::new(format!("Start the download runtime: {error}")))?;
        let client = nora_network::http_client_builder()
            .map_err(|error| Error::new(format!("Configure TLS: {error}")))?
            .redirect(reqwest::redirect::Policy::none())
            .timeout(DOWNLOAD_TIMEOUT)
            .build()
            .map_err(|error| Error::new(format!("Configure HTTP: {error}")))?;
        Ok(Self { runtime, client })
    }
}

impl Fetch for Network {
    fn fetch(&self, url: &Url, limit: usize) -> Result<Reply> {
        self.runtime.block_on(async {
            let failed =
                |error: reqwest::Error| Error::new(format!("Release download failed: {error}"));
            let mut response = self.client.get(url.clone()).send().await.map_err(failed)?;
            let status = response.status().as_u16();
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            if response.content_length().unwrap_or(0) > limit as u64 {
                return Err(Error::new("Release archive exceeds 256 MiB"));
            }
            let mut body = Vec::new();
            if status == 200 {
                while let Some(chunk) = response.chunk().await.map_err(failed)? {
                    body.extend_from_slice(&chunk);
                    if body.len() > limit {
                        return Err(Error::new("Release archive exceeds 256 MiB"));
                    }
                }
            }
            Ok(Reply {
                status,
                location,
                body,
            })
        })
    }
}

/// Downloads a release archive from GitHub, following redirects only to
/// GitHub's release asset hosts.
pub fn download_release(url: &str, fetcher: &impl Fetch) -> Result<Vec<u8>> {
    let mut target = Url::parse(url).map_err(|_| Error::new("Unexpected release download host"))?;
    for _ in 0..=MAXIMUM_REDIRECTS {
        let allowed = target.scheme() == "https"
            && target.username().is_empty()
            && target.password().is_none()
            && target.port().is_none()
            && target
                .host_str()
                .is_some_and(|host| RELEASE_HOSTS.contains(&host));
        if !allowed {
            return Err(Error::new("Unexpected release download host"));
        }
        let reply = fetcher.fetch(&target, MAXIMUM_ARCHIVE_BYTES)?;
        if matches!(reply.status, 301 | 302 | 303 | 307 | 308) {
            let location = reply
                .location
                .ok_or_else(|| Error::new("Download redirect has no location"))?;
            target = target
                .join(&location)
                .map_err(|_| Error::new("Unexpected release download host"))?;
            continue;
        }
        if reply.status != 200 {
            return Err(Error::new(format!(
                "Release download returned HTTP {}",
                reply.status
            )));
        }
        return Ok(reply.body);
    }
    Err(Error::new("Too many release download redirects"))
}

/// Downloads and checks every entry that is new or changed since `baseline`.
pub fn verify_downloads(
    registry: &Value,
    baseline: Option<&Value>,
    fetcher: &impl Fetch,
) -> Result<()> {
    let plugins = |registry: &Value| registry["plugins"].as_array().cloned().unwrap_or_default();
    let previous: HashSet<String> = baseline
        .map(plugins)
        .unwrap_or_default()
        .iter()
        .map(canonical)
        .collect();
    let changed: Vec<Value> = plugins(registry)
        .into_iter()
        .filter(|entry| !previous.contains(&canonical(entry)))
        .collect();
    if changed.len() > MAXIMUM_CHANGED_PLUGINS {
        return Err(Error::new(
            "Validate at most 100 new or changed plugins per submission",
        ));
    }
    let mut total = 0;
    for entry in &changed {
        let archive = download_release(entry["downloadURL"].as_str().unwrap_or_default(), fetcher)?;
        total += archive.len();
        if total > MAXIMUM_SUBMISSION_BYTES {
            return Err(Error::new("Submission downloads exceed 1 GiB"));
        }
        verify_archive(entry, &archive)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Scripted(RefCell<Vec<(String, Reply)>>);

    impl Fetch for Scripted {
        fn fetch(&self, url: &Url, _limit: usize) -> Result<Reply> {
            let (expected, reply) = self.0.borrow_mut().remove(0);
            assert_eq!(url.host_str(), Some(expected.as_str()));
            Ok(reply)
        }
    }

    fn reply(status: u16, location: Option<&str>, body: &[u8]) -> Reply {
        Reply {
            status,
            location: location.map(str::to_owned),
            body: body.to_vec(),
        }
    }

    const RELEASE: &str =
        "https://github.com/nora-tests/plugin/releases/download/v1.2.3/plugin.zip";

    #[test]
    fn redirects_stay_on_release_hosts() {
        let fetcher = Scripted(RefCell::new(vec![
            (
                "github.com".into(),
                reply(
                    302,
                    Some("https://release-assets.githubusercontent.com/example.zip?token=public"),
                    b"",
                ),
            ),
            (
                "release-assets.githubusercontent.com".into(),
                reply(200, None, b"archive"),
            ),
        ]));
        assert_eq!(download_release(RELEASE, &fetcher).unwrap(), b"archive");
        let private = Scripted(RefCell::new(vec![(
            "github.com".into(),
            reply(302, Some("http://127.0.0.1/private"), b""),
        )]));
        assert!(download_release(RELEASE, &private).is_err());
        let missing = Scripted(RefCell::new(vec![(
            "github.com".into(),
            reply(404, None, b""),
        )]));
        assert!(download_release(RELEASE, &missing).is_err());
    }

    #[test]
    fn reviewed_entries_are_not_downloaded_again() {
        let registry =
            serde_json::json!({ "plugins": [{ "downloadURL": RELEASE, "sha256": "1" }] });
        let unused = Scripted(RefCell::new(Vec::new()));
        verify_downloads(&registry, Some(&registry), &unused).unwrap();
        assert!(verify_downloads(
            &registry,
            None,
            &Scripted(RefCell::new(vec![(
                "github.com".into(),
                reply(404, None, b"")
            )]))
        )
        .is_err());
    }
}
