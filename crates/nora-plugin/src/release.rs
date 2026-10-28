use crate::files;
use crate::options::Options;
use nora_plugin_manifest::{
    encode_uri_component as encode, format_registry, repository_path, sha256_hex as sha256,
    validate_entry, validate_native_manifest, verify_archive, Error, Result, MAXIMUM_ARCHIVE_BYTES,
    MAXIMUM_MANIFEST_BYTES,
};
use serde_json::{json, Value};
use std::path::Path;

/// Writes the marketplace entry, manifest, and checksum that a signed
/// release publishes beside its archive.
pub fn release_entry(options: &Options) -> Result<()> {
    let manifest = files::read_json(options.required("--manifest")?, MAXIMUM_MANIFEST_BYTES)?;
    validate_native_manifest(&manifest)?;
    let archive_path = Path::new(options.required("--archive")?);
    let archive = files::read_limited(
        archive_path,
        MAXIMUM_ARCHIVE_BYTES,
        "Release archive exceeds 256 MiB",
    )?;
    let filename = archive_name(archive_path)?;
    let repository = repository_path(Some(&Value::from(options.required("--repository")?)))?;
    let tag = options.required("--tag")?;
    let digest = sha256(&archive);
    let entry = validate_entry(&json!({
        "manifest": manifest,
        "repositoryURL": format!("https://github.com/{repository}"),
        "releaseTag": tag,
        "teamIdentifier": options.required("--team")?,
        "sha256": digest,
        "downloadURL": format!(
            "https://github.com/{repository}/releases/download/{}/{}",
            encode(tag),
            encode(filename)
        ),
    }))?;
    verify_archive(&entry, &archive)?;
    let output = Path::new(options.required("--output")?);
    let name = manifest["name"].clone();
    let marketplace = json!({ "schemaVersion": 1, "name": name, "plugins": [entry] });
    files::create(
        output.join("marketplace.json"),
        format_registry(&marketplace).as_bytes(),
    )?;
    files::create(
        output.join("nora-plugin.json"),
        pretty(&manifest).as_bytes(),
    )?;
    files::create(
        output.join("SHA256SUMS"),
        format!("{digest}  {filename}\n").as_bytes(),
    )?;
    println!(
        "Prepared release metadata for {}@{}",
        manifest["id"].as_str().unwrap_or_default(),
        manifest["version"].as_str().unwrap_or_default()
    );
    Ok(())
}

/// Writes a development marketplace that installs a local archive by its
/// file name. Nora accepts it only from a local folder, never a registry.
pub fn local_marketplace(options: &Options) -> Result<()> {
    let manifest = files::read_json(options.required("--manifest")?, MAXIMUM_MANIFEST_BYTES)?;
    validate_native_manifest(&manifest)?;
    let archive_path = Path::new(options.required("--archive")?);
    let archive = files::read_limited(
        archive_path,
        MAXIMUM_ARCHIVE_BYTES,
        "Release archive exceeds 256 MiB",
    )?;
    let marketplace = json!({
        "schemaVersion": 1,
        "name": options.optional("--name").unwrap_or("Local Nora Plugins"),
        "plugins": [{ "manifest": manifest, "downloadURL": archive_name(archive_path)?, "sha256": sha256(&archive) }],
    });
    let output = options.required("--output")?;
    files::write(output, pretty(&marketplace).as_bytes())?;
    println!("Marketplace: {output}");
    Ok(())
}

fn archive_name(path: &Path) -> Result<&str> {
    path.file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| Error::new("The release archive needs a UTF-8 file name"))
}

fn pretty(value: &Value) -> String {
    format!(
        "{}\n",
        serde_json::to_string_pretty(value).unwrap_or_default()
    )
}
