use crate::error::{ensure, Error, Result};
use crate::native::validate_native_manifest;
use crate::text::{array, is_one, object, text};
use regex::Regex;
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};
use std::sync::LazyLock;
use url::Url;

pub const MAXIMUM_REGISTRY_PLUGINS: usize = 1000;

static REPOSITORY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^/[A-Za-z0-9][A-Za-z0-9-]*/[A-Za-z0-9][A-Za-z0-9._-]*$").unwrap()
});
static TEAM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Z0-9]{10}$").unwrap());
static DIGEST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-f0-9]{64}$").unwrap());
static ASSET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]*\.zip$").unwrap());

/// The `owner/repository` of a canonical public GitHub repository URL.
pub fn repository_path(value: Option<&Value>) -> Result<String> {
    let value = value
        .and_then(Value::as_str)
        .ok_or_else(|| Error::new("repositoryURL is required for public plugins"))?;
    let url = Url::parse(value).ok();
    let path = url
        .as_ref()
        .filter(|url| plain_github(url, &["github.com"]) && REPOSITORY.is_match(url.path()))
        .map(Url::path)
        .filter(|path| !path.ends_with(".git"))
        .ok_or_else(|| {
            Error::new(
                "repositoryURL must be a canonical public https://github.com/owner/repository URL",
            )
        })?;
    Ok(path[1..].to_owned())
}

/// HTTPS to one of `hosts`, with no port, credentials, query, or fragment.
pub(crate) fn plain_github(url: &Url, hosts: &[&str]) -> bool {
    url.scheme() == "https"
        && url.host_str().is_some_and(|host| hosts.contains(&host))
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
}

/// Checks a marketplace entry and returns it in the registry's field order.
pub fn validate_entry(value: &Value) -> Result<Value> {
    let entry = object(value, "Registry entry")?;
    let manifest = validate_native_manifest(entry.get("manifest").unwrap_or(&Value::Null))?;
    let version = manifest["version"].as_str().unwrap_or_default();
    let repository = repository_path(entry.get("repositoryURL"))?;
    let tag = entry
        .get("releaseTag")
        .and_then(Value::as_str)
        .unwrap_or_default();
    ensure(
        tag == version || tag == format!("v{version}"),
        "releaseTag must match the exact manifest version, optionally prefixed by v",
    )?;
    let team = entry
        .get("teamIdentifier")
        .and_then(Value::as_str)
        .unwrap_or_default();
    ensure(
        TEAM.is_match(team),
        "Native teamIdentifier must be a 10-character Apple Developer Team ID",
    )?;
    let digest = entry
        .get("sha256")
        .and_then(Value::as_str)
        .unwrap_or_default();
    ensure(
        DIGEST.is_match(digest) && digest.bytes().any(|byte| byte != b'0'),
        "sha256 must be a real lowercase SHA-256 digest",
    )?;
    let download = entry
        .get("downloadURL")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::new("downloadURL is required"))?;
    let prefix = format!(
        "/{repository}/releases/download/{}/",
        encode_uri_component(tag)
    );
    let asset = Url::parse(download)
        .ok()
        .filter(|url| plain_github(url, &["github.com"]))
        .and_then(|url| url.path().strip_prefix(&prefix).map(str::to_owned))
        .ok_or_else(|| {
            Error::new("downloadURL must point to the declared repository and exact release tag")
        })?;
    let asset = decode_uri_component(&asset);
    ensure(
        asset.is_some_and(|asset| ASSET.is_match(&asset)),
        "Release asset must be a ZIP with a simple filename",
    )?;
    Ok(json!({
        "manifest": manifest,
        "repositoryURL": entry["repositoryURL"],
        "releaseTag": tag,
        "teamIdentifier": team,
        "downloadURL": download,
        "sha256": digest,
    }))
}

/// Checks a whole marketplace and returns it with its plugins in ID order.
pub fn validate_registry(value: &Value) -> Result<Value> {
    let registry = object(value, "Marketplace")?;
    ensure(
        is_one(registry.get("schemaVersion")),
        "Only marketplace schemaVersion 1 is supported",
    )?;
    let name = text(registry.get("name"), "marketplace name", 200)?;
    let mut plugins = array(registry.get("plugins"), "plugins", MAXIMUM_REGISTRY_PLUGINS)?
        .iter()
        .map(validate_entry)
        .collect::<Result<Vec<_>>>()?;
    plugins.sort_by(|left, right| compare_ids(plugin_id(left), plugin_id(right)));
    let ids: HashSet<_> = plugins.iter().map(plugin_id).collect();
    ensure(
        ids.len() == plugins.len(),
        "Duplicate plugin IDs in registry",
    )?;
    Ok(json!({ "schemaVersion": 1, "name": name, "plugins": plugins }))
}

fn plugin_id(entry: &Value) -> &str {
    entry["manifest"]["id"].as_str().unwrap_or_default()
}

/// Plugin IDs are ASCII, so English collation is the case-insensitive order
/// with lower case first among otherwise equal IDs.
fn compare_ids(left: &str, right: &str) -> Ordering {
    left.to_ascii_lowercase()
        .cmp(&right.to_ascii_lowercase())
        .then_with(|| {
            left.bytes()
                .zip(right.bytes())
                .find(|(left, right)| left != right)
                .map_or(Ordering::Equal, |(left, right)| right.cmp(&left))
        })
}

pub fn format_registry(registry: &Value) -> String {
    let mut output = serde_json::to_string_pretty(registry).unwrap_or_default();
    output.push('\n');
    output
}

/// A key-sorted encoding, for comparing two values regardless of key order.
pub fn canonical(value: &Value) -> String {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
            Value::Object(fields) => {
                let fields: BTreeMap<_, _> = fields
                    .iter()
                    .map(|(key, value)| (key.clone(), sorted(value)))
                    .collect();
                Value::Object(fields.into_iter().collect())
            }
            value => value.clone(),
        }
    }
    sorted(value).to_string()
}

/// JavaScript's `encodeURIComponent`, which GitHub release URLs follow.
pub fn encode_uri_component(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => (byte as char).to_string(),
            byte => format!("%{byte:02X}"),
        })
        .collect()
}

fn decode_uri_component(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}
