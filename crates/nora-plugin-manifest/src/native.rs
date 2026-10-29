use crate::error::{ensure, Error, Result};
use crate::text::{
    array, identifier, is_one, is_semantic_version, object, relative_resource_path, text,
};
use regex::Regex;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::LazyLock;

pub const MAXIMUM_MANIFEST_BYTES: usize = 512 * 1024;
const MAXIMUM_SCHEMA_BYTES: usize = 64 * 1024;
const PERMISSIONS: [&str; 6] = [
    "hostInfo",
    "notifications",
    "network",
    "credentials",
    "openExternal",
    "chat",
];

static HOST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)*$")
        .unwrap()
});
static ICON_EXTENSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\.(svg|png|jpg|jpeg)$").unwrap());

/// Checks a native plugin's `nora-plugin.json` and returns it unchanged.
pub fn validate_native_manifest(value: &Value) -> Result<&Value> {
    let manifest = object(value, "Plugin manifest")?;
    ensure(
        is_one(manifest.get("apiVersion")) && is_one(manifest.get("schemaVersion")),
        "Requires schemaVersion 1 and apiVersion 1",
    )?;
    ensure(
        !manifest.contains_key("runtime") && !manifest.contains_key("entryPoint"),
        "Native plugins do not support runtime or entryPoint fields",
    )?;
    let plugin_id = identifier(manifest.get("id"), "id")?;
    if manifest.contains_key("iconPath") {
        let icon = relative_resource_path(manifest.get("iconPath"), "iconPath")?;
        ensure(
            !icon.to_lowercase().starts_with("contents/resources/"),
            "iconPath must be relative to Contents/Resources",
        )?;
        ensure(
            ICON_EXTENSION.is_match(icon),
            "iconPath must name an SVG, PNG, or JPEG file inside the plugin",
        )?;
    }
    identifier(
        manifest.get("extensionBundleIdentifier"),
        "extensionBundleIdentifier",
    )?;
    let domains = match manifest.get("networkDomains") {
        None | Some(Value::Null) => &[][..],
        domains => array(domains, "networkDomains", 64)?,
    };
    let unique_domains: HashSet<_> = domains.iter().filter_map(Value::as_str).collect();
    ensure(
        unique_domains.len() == domains.len()
            && domains
                .iter()
                .filter_map(Value::as_str)
                .all(|domain| domain.len() <= 253 && HOST.is_match(domain)),
        "networkDomains must contain unique exact lowercase hosts without schemes, ports, or wildcards",
    )?;
    text(manifest.get("name"), "name", 128)?;
    text(manifest.get("publisher"), "publisher", 128)?;
    text(manifest.get("summary"), "summary", 2048)?;
    let version = text(manifest.get("version"), "version", 128)?;
    ensure(
        is_semantic_version(version),
        "version must be a strict semantic version",
    )?;
    let permissions = array(
        manifest.get("permissions"),
        "permissions",
        PERMISSIONS.len(),
    )?;
    let names: Vec<_> = permissions.iter().filter_map(Value::as_str).collect();
    ensure(
        names.len() == permissions.len()
            && names.iter().collect::<HashSet<_>>().len() == names.len()
            && names.iter().all(|name| PERMISSIONS.contains(name)),
        "Unknown or duplicate plugin permission",
    )?;
    ensure(
        domains.is_empty() || names.contains(&"network"),
        "networkDomains requires the network permission",
    )?;
    let mut contributions = HashSet::new();
    let mut contribution = |value: &'_ Value| -> Result<()> {
        let item = object(value, "Contribution")?;
        let id = identifier(item.get("id"), "contribution id")?;
        ensure(
            id.starts_with(&format!("{plugin_id}.")) && contributions.insert(id.to_owned()),
            "Contribution IDs must be unique and belong to the plugin",
        )?;
        text(item.get("title"), "contribution title", 128)?;
        Ok(())
    };
    for panel in array(manifest.get("panels"), "panels", 32)? {
        contribution(panel)?;
        text(panel.get("systemImage"), "panel systemImage", 128)?;
    }
    for command in array(manifest.get("commands"), "commands", 128)? {
        contribution(command)?;
        text(command.get("description"), "command description", 4096)?;
        let schema = command.get("inputSchemaJSON").and_then(Value::as_str);
        ensure(
            schema.is_some_and(|schema| schema.len() <= MAXIMUM_SCHEMA_BYTES),
            "Command schema exceeds 64 KiB or is not a string",
        )?;
        let schema: Value = serde_json::from_str(schema.unwrap_or_default())
            .map_err(|_| Error::new("Command input schema must be JSON"))?;
        ensure(
            schema.as_object().and_then(|schema| schema.get("type"))
                == Some(&Value::from("object")),
            "Command input schema must describe a JSON object",
        )?;
        ensure(
            command.get("readOnly").is_some_and(Value::is_boolean),
            "Command readOnly must be a boolean",
        )?;
    }
    let encoded = serde_json::to_vec(value).map_err(|error| Error::new(error.to_string()))?;
    ensure(
        encoded.len() <= MAXIMUM_MANIFEST_BYTES,
        "Plugin manifest exceeds 512 KiB",
    )?;
    Ok(value)
}
