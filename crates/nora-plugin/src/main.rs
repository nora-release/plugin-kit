//! `nora-plugin`: validate, package, and publish Nora plugins and
//! marketplaces.

mod bundle;
mod download;
mod files;
mod options;
mod release;
mod scaffold;

use nora_plugin_manifest::{
    format_registry, validate_icon_data, validate_native_manifest, validate_registry, Error,
    Result, MAXIMUM_ICON_BYTES, MAXIMUM_MANIFEST_BYTES,
};
use options::Options;
use std::process::ExitCode;

const COMMANDS: &str =
    "Commands: generate, validate, validate-plugin, validate-manifest, validate-icon, release-entry, \
    local-marketplace, bundle, init-registry, init-author, sdk-version";

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match run(&arguments) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run(arguments: &[String]) -> Result<()> {
    let Some((command, rest)) = arguments.split_first() else {
        return Err(Error::new(COMMANDS));
    };
    let options = Options::parse(rest)?;
    match command.as_str() {
        "validate-manifest" => {
            let manifest =
                files::read_json(options.required("--manifest")?, MAXIMUM_MANIFEST_BYTES)?;
            validate_native_manifest(&manifest)?;
            println!("Validated plugin manifest");
        }
        "validate-plugin" => {
            let directory = std::path::Path::new(options.required("--directory")?);
            let manifest = files::read_limited(
                directory.join(nora_plugin_manifest::package::MANIFEST_FILE),
                MAXIMUM_MANIFEST_BYTES,
                "Plugin manifest exceeds 512 KiB",
            )?;
            let manifest = nora_plugin_manifest::PluginManifest::parse(&manifest)?;
            manifest.validate_files(directory)?;
            println!("Validated {}@{}", manifest.id, manifest.version);
        }
        "validate-icon" => {
            let path = options.required("--icon")?;
            let icon =
                files::read_limited(path, MAXIMUM_ICON_BYTES, "Plugin icon exceeds 512 KiB")?;
            validate_icon_data(&icon, path)?;
        }
        "generate" | "validate" => {
            let registry = validate_registry(&files::read_json(
                options.required("--registry")?,
                files::JSON_LIMIT,
            )?)?;
            if options.has("--verify-downloads") {
                let baseline = match options.optional("--baseline") {
                    Some(path) => Some(validate_registry(&files::read_json(
                        path,
                        files::JSON_LIMIT,
                    )?)?),
                    None => None,
                };
                download::verify_downloads(
                    &registry,
                    baseline.as_ref(),
                    &download::Network::new()?,
                )?;
            }
            let output = format_registry(&registry);
            if command == "generate" {
                files::write(options.required("--output")?, output.as_bytes())?;
            } else {
                let catalog = files::read_text(options.required("--catalog")?)?;
                if catalog != output {
                    return Err(Error::new(
                        "marketplace.json is stale; regenerate it from registry.json",
                    ));
                }
            }
            let count = registry["plugins"].as_array().map_or(0, Vec::len);
            println!("Validated {count} plugin entries");
        }
        "release-entry" => release::release_entry(&options)?,
        "local-marketplace" => release::local_marketplace(&options)?,
        "bundle" => bundle::bundle(&options)?,
        "init-registry" => scaffold::init_registry(&options)?,
        "init-author" => scaffold::init_author(&options)?,
        "sdk-version" => scaffold::check_sdk_version(&options)?,
        _ => return Err(Error::new(COMMANDS)),
    }
    Ok(())
}
