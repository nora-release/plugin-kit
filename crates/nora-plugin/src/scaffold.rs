use crate::files;
use crate::options::Options;
use nora_plugin_manifest::{is_semantic_version, Error, Result};
use regex::Regex;
use serde_json::Value;
use std::path::Path;
use std::sync::LazyLock;
use url::Url;

/// The plugin-kit release this tool was built from; scaffolds pin it.
const VERSION: &str = env!("CARGO_PKG_VERSION");
const VERSION_PLACEHOLDER: &str = "__PLUGIN_KIT_VERSION__";
const SDK_URL: &str = "https://github.com/nora-release/plugin-kit.git";
const LICENSE: &str = include_str!("../../../LICENSE");
const SDK_VERSION_FILE: &str = include_str!("../../../sdk-version.json");

const REGISTRY_FILES: [(&str, &str); 3] = [
    (
        "registry.json",
        include_str!("../../../templates/registry/registry.json"),
    ),
    (
        "marketplace.json",
        include_str!("../../../templates/registry/marketplace.json"),
    ),
    (
        ".github/workflows/validate-marketplace.yml",
        include_str!("../../../templates/registry/.github/workflows/validate-marketplace.yml"),
    ),
];
const AUTHOR_FILES: [(&str, &str); 8] = [
    (
        "Package.swift",
        include_str!("../../../templates/author/Package.swift"),
    ),
    (
        ".gitignore",
        include_str!("../../../templates/author/gitignore"),
    ),
    (
        "nora-plugin.json",
        include_str!("../../../templates/author/nora-plugin.json"),
    ),
    (
        "Sources/PluginHost/PluginHost.swift",
        include_str!("../../../templates/author/Sources/PluginHost/PluginHost.swift"),
    ),
    (
        "Sources/PluginExtension/NativePlugin.swift",
        include_str!("../../../templates/author/Sources/PluginExtension/NativePlugin.swift"),
    ),
    (
        "plugin-release.json",
        include_str!("../../../templates/author/plugin-release.json"),
    ),
    (
        ".github/workflows/release-plugin.yml",
        include_str!("../../../templates/author/.github/workflows/release-plugin.yml"),
    ),
    (
        "scripts/publish-native-plugin.sh",
        include_str!("../../../scripts/publish-native-plugin.sh"),
    ),
];

static SDK_DEPENDENCY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"\.package\(url: "https://github\.com/nora-release/plugin-kit\.git", exact: "[^"]*"\)"#,
    )
    .unwrap()
});
static PACKAGE_IDENTITY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9][a-z0-9._-]*$").unwrap());

pub fn init_registry(options: &Options) -> Result<()> {
    let directory = Path::new(options.required("--directory")?);
    for (path, contents) in REGISTRY_FILES {
        files::create(directory.join(path), pinned(contents).as_bytes())?;
    }
    files::create(directory.join("NoraPluginKit-LICENSE"), LICENSE.as_bytes())?;
    println!("Created registry files in {}", directory.display());
    Ok(())
}

pub fn init_author(options: &Options) -> Result<()> {
    let directory = Path::new(options.required("--directory")?);
    let (dependency, identity) = sdk_dependency(options)?;
    for (path, contents) in AUTHOR_FILES {
        let mut contents = pinned(contents);
        if path == "Package.swift" {
            contents = SDK_DEPENDENCY
                .replace(&contents, dependency.as_str())
                .into_owned();
            contents = contents.replace(
                r#"package: "plugin-kit""#,
                &format!("package: {identity:?}"),
            );
        }
        files::create(directory.join(path), contents.as_bytes())?;
        if path.ends_with(".sh") {
            make_executable(&directory.join(path))?;
        }
    }
    files::create(directory.join("NoraPluginKit-LICENSE"), LICENSE.as_bytes())?;
    println!("Created author release files in {}", directory.display());
    Ok(())
}

/// The Swift package dependency on plugin-kit and its package identity.
fn sdk_dependency(options: &Options) -> Result<(String, String)> {
    if let Some(path) = options.optional("--sdk-path") {
        if options.has("--sdk-url") || options.has("--sdk-version") {
            return Err(Error::new(
                "--sdk-path cannot be combined with --sdk-url or --sdk-version",
            ));
        }
        let path =
            std::fs::canonicalize(path).map_err(|error| files::io(Path::new(path), error))?;
        if !path.join("Package.swift").is_file() {
            return Err(Error::new("--sdk-path must contain a Swift package"));
        }
        let path = path.to_string_lossy().into_owned();
        return Ok((
            format!(".package(name: \"plugin-kit\", path: {path:?})"),
            "plugin-kit".into(),
        ));
    }
    let url = options.optional("--sdk-url").unwrap_or(SDK_URL);
    let version = options.optional("--sdk-version").unwrap_or(VERSION);
    let parsed = Url::parse(url).ok().filter(|parsed| {
        matches!(parsed.scheme(), "https" | "file")
            && parsed.username().is_empty()
            && parsed.password().is_none()
            && parsed.query().is_none()
            && parsed.fragment().is_none()
    });
    let parsed = parsed.ok_or_else(|| {
        Error::new("--sdk-url must be an HTTPS or local file Git URL without credentials")
    })?;
    if !is_semantic_version(version) {
        return Err(Error::new(
            "--sdk-version must be an exact semantic version",
        ));
    }
    let identity = parsed
        .path_segments()
        .and_then(|segments| segments.rev().find(|segment| !segment.is_empty()))
        .map(|name| name.to_lowercase().trim_end_matches(".git").to_owned())
        .filter(|name| PACKAGE_IDENTITY.is_match(name))
        .ok_or_else(|| Error::new("--sdk-url must identify a named Git repository"))?;
    Ok((
        format!(".package(url: {url:?}, exact: {version:?})"),
        identity,
    ))
}

fn pinned(template: &str) -> String {
    template.replace(VERSION_PLACEHOLDER, VERSION)
}

fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .map_err(|error| files::io(path, error))
}

/// Checks that `sdk-version.json`, this tool, and an optional release tag
/// all name the same plugin-kit version.
pub fn check_sdk_version(options: &Options) -> Result<()> {
    let metadata: Value = match options.optional("--file") {
        Some(path) => files::read_json(path, files::JSON_LIMIT)?,
        None => {
            serde_json::from_str(SDK_VERSION_FILE).map_err(|error| Error::new(error.to_string()))?
        }
    };
    if metadata["schemaVersion"].as_f64() != Some(1.0) {
        return Err(Error::new("Unsupported SDK version file schema."));
    }
    if metadata["apiVersion"].as_f64() != Some(1.0) {
        return Err(Error::new("Unsupported plugin API version."));
    }
    let version = metadata["version"].as_str().unwrap_or_default();
    if version.trim() != version || !is_semantic_version(version) {
        return Err(Error::new("SDK version must be a strict semantic version."));
    }
    if version != VERSION {
        return Err(Error::new(format!(
            "sdk-version.json says {version} but the Rust workspace is {VERSION}."
        )));
    }
    if let Some(tag) = options.optional("--tag") {
        if tag != format!("v{version}") {
            return Err(Error::new("Release tag must match sdk-version.json."));
        }
    }
    println!("plugin-kit {version}");
    Ok(())
}
