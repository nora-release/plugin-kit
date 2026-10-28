use crate::files;
use crate::options::Options;
use nora_plugin_manifest::{validate_native_manifest, Error, Result, MAXIMUM_MANIFEST_BYTES};
use plist::{Dictionary, Value as Plist};
use regex::Regex;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;

const EXTENSION_POINT: &str = "com.nora.plugin.native";
const ARCHITECTURES: [&str; 2] = ["arm64", "x86_64"];

static PRODUCT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_-]*$").unwrap());
static PRINCIPAL_CLASS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*\.[A-Za-z_][A-Za-z0-9_]*$").unwrap());
static SYSTEM_VERSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\d+\.\d+(?:\.\d+)?$").unwrap());

/// `plugin-release.json`: how to build the containing app and its native
/// extension from a Swift package.
struct Release {
    package: PathBuf,
    manifest_path: PathBuf,
    manifest: Value,
    host_product: String,
    extension_product: String,
    principal_class: String,
    minimum_system: String,
    host_resources: Vec<PathBuf>,
    extension_resources: Vec<PathBuf>,
}

impl Release {
    fn read(path: &Path) -> Result<Self> {
        let config_path = fs::canonicalize(path).map_err(|error| files::io(path, error))?;
        let config = files::read_json(&config_path, files::JSON_LIMIT)?;
        if config["schemaVersion"].as_f64() != Some(1.0) {
            return Err(Error::new(
                "Only release config schemaVersion 1 is supported",
            ));
        }
        let root = config_path.parent().unwrap_or(Path::new("/")).to_path_buf();
        let field = |key: &str| {
            config[key]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| Error::new(format!("{key} is required")))
        };
        let resources = |key: &str| -> Vec<PathBuf> {
            config[key]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(|relative| root.join(relative))
                .collect()
        };
        let manifest_path = root.join(field("manifestPath")?);
        let manifest = files::read_json(&manifest_path, MAXIMUM_MANIFEST_BYTES)?;
        validate_native_manifest(&manifest)?;
        let release = Self {
            package: root.join(field("packagePath")?),
            manifest_path,
            manifest,
            host_product: field("hostProduct")?,
            extension_product: field("extensionProduct")?,
            principal_class: field("extensionPrincipalClass")?,
            minimum_system: config["minimumSystemVersion"]
                .as_str()
                .unwrap_or("14.0")
                .to_owned(),
            host_resources: resources("hostResources"),
            extension_resources: resources("extensionResources"),
        };
        for (key, product) in [
            ("hostProduct", &release.host_product),
            ("extensionProduct", &release.extension_product),
        ] {
            if !PRODUCT.is_match(product) {
                return Err(Error::new(format!("Invalid {key}")));
            }
        }
        if !PRINCIPAL_CLASS.is_match(&release.principal_class) {
            return Err(Error::new("extensionPrincipalClass must be Module.Class"));
        }
        let major = release
            .minimum_system
            .split('.')
            .next()
            .and_then(|major| major.parse::<u32>().ok());
        if !SYSTEM_VERSION.is_match(&release.minimum_system) || major.is_none_or(|major| major < 14)
        {
            return Err(Error::new(
                "minimumSystemVersion must be macOS 14.0 or later",
            ));
        }
        Ok(release)
    }

    fn text(&self, key: &str) -> &str {
        self.manifest[key].as_str().unwrap_or_default()
    }
}

/// Builds the containing app with its native extension, ready to sign.
/// A release build is universal; `--debug` builds this Mac's architecture
/// for local testing.
pub fn bundle(options: &Options) -> Result<()> {
    let release = Release::read(Path::new(options.required("--config")?))?;
    if let Some(tag) = options.optional("--tag") {
        if tag != format!("v{}", release.text("version")) {
            return Err(Error::new(
                "Release tag must match v + the plugin manifest version",
            ));
        }
    }
    let debug = options.has("--debug");
    let output = Path::new(options.required("--output")?);
    fs::create_dir_all(output).map_err(|error| files::io(output, error))?;
    let output = fs::canonicalize(output).map_err(|error| files::io(output, error))?;
    let app = output.join(format!("{}.app", release.text("id")));
    if app.exists() {
        return Err(Error::new("Refusing to overwrite an existing application"));
    }
    let mut build = vec![
        "build".to_owned(),
        "--package-path".to_owned(),
        path_text(&release.package)?,
    ];
    if debug {
        build.extend(["--configuration", "debug"].map(str::to_owned));
    } else {
        build.extend(
            [
                "--configuration",
                "release",
                "--arch",
                "arm64",
                "--arch",
                "x86_64",
            ]
            .map(str::to_owned),
        );
    }
    if let Some(scratch) = options.optional("--scratch-path") {
        build.extend(["--scratch-path".to_owned(), scratch.to_owned()]);
    }
    for product in [&release.host_product, &release.extension_product] {
        run_swift(&build, &["--product", product])?;
    }
    let binaries = PathBuf::from(
        run_swift(&build, &["--show-bin-path"])?
            .lines()
            .next_back()
            .unwrap_or_default()
            .trim(),
    );
    let extension = app.join("Contents/Extensions/NativePlugin.appex");
    for bundle in [&app, &extension] {
        for folder in ["Contents/MacOS", "Contents/Resources"] {
            let folder = bundle.join(folder);
            fs::create_dir_all(&folder).map_err(|error| files::io(&folder, error))?;
        }
    }
    let host = info(
        &release,
        &release.host_product,
        release.text("id"),
        [
            ("CFBundlePackageType", Plist::from("APPL")),
            ("NSPrincipalClass", Plist::from("NSApplication")),
            ("LSUIElement", Plist::Boolean(true)),
        ],
    );
    let mut attributes = Dictionary::new();
    attributes.insert("EXExtensionPointIdentifier".into(), EXTENSION_POINT.into());
    attributes.insert(
        "EXExtensionPrincipalClass".into(),
        release.principal_class.clone().into(),
    );
    let extension_info = info(
        &release,
        &release.extension_product,
        release.text("extensionBundleIdentifier"),
        [
            ("CFBundlePackageType", Plist::from("XPC!")),
            ("EXAppExtensionAttributes", Plist::Dictionary(attributes)),
        ],
    );
    for (bundle, product, info) in [
        (&app, &release.host_product, host),
        (&extension, &release.extension_product, extension_info),
    ] {
        let executable = bundle.join("Contents/MacOS").join(product);
        copy(&binaries.join(product), &executable)?;
        if !debug {
            let architectures = run("lipo", &["-archs".to_owned(), path_text(&executable)?])?;
            let architectures: Vec<&str> = architectures.split_whitespace().collect();
            if !ARCHITECTURES
                .iter()
                .all(|architecture| architectures.contains(architecture))
            {
                return Err(Error::new(format!(
                    "{product} must support arm64 and x86_64"
                )));
            }
        }
        let path = bundle.join("Contents/Info.plist");
        Plist::Dictionary(info)
            .to_file_xml(&path)
            .map_err(|error| Error::new(format!("{}: {error}", path.display())))?;
    }
    copy(
        &release.manifest_path,
        &app.join("Contents/Resources/nora-plugin.json"),
    )?;
    for (resources, bundle) in [
        (&release.host_resources, &app),
        (&release.extension_resources, &extension),
    ] {
        for source in resources {
            let source = fs::canonicalize(source).map_err(|error| files::io(source, error))?;
            let name = source
                .file_name()
                .ok_or_else(|| Error::new("Resource needs a name"))?;
            let target = bundle.join("Contents/Resources").join(name);
            if target.exists() {
                return Err(Error::new(format!(
                    "Resource already exists: {}",
                    name.to_string_lossy()
                )));
            }
            copy_tree(&source, &target)?;
        }
    }
    println!(
        "Built {} application: {}",
        if debug { "debug" } else { "universal" },
        app.display()
    );
    Ok(())
}

fn info<const N: usize>(
    release: &Release,
    executable: &str,
    identifier: &str,
    extra: [(&str, Plist); N],
) -> Dictionary {
    let version = release.text("version");
    let bundle_version = version.split(['-', '+']).next().unwrap_or(version);
    let mut info = Dictionary::new();
    for (key, value) in [
        ("CFBundleDevelopmentRegion", "en"),
        ("CFBundleInfoDictionaryVersion", "6.0"),
        ("CFBundleShortVersionString", bundle_version),
        ("CFBundleVersion", bundle_version),
        ("LSMinimumSystemVersion", &release.minimum_system),
        ("CFBundleIdentifier", identifier),
        ("CFBundleExecutable", executable),
        ("CFBundleName", release.text("name")),
    ] {
        info.insert(key.into(), value.into());
    }
    for (key, value) in extra {
        info.insert(key.into(), value);
    }
    info
}

fn run_swift(build: &[String], extra: &[&str]) -> Result<String> {
    let mut arguments = build.to_vec();
    arguments.extend(extra.iter().map(|value| (*value).to_owned()));
    run("swift", &arguments)
}

fn run(program: &str, arguments: &[String]) -> Result<String> {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .map_err(|error| Error::new(format!("Run {program}: {error}")))?;
    if !output.status.success() {
        return Err(Error::new(format!(
            "{program} {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn path_text(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| Error::new(format!("Path is not UTF-8: {}", path.display())))
}

fn copy(source: &Path, target: &Path) -> Result<()> {
    fs::copy(source, target)
        .map(|_| ())
        .map_err(|error| files::io(source, error))
}

/// Copies a file or folder, keeping symbolic links as links.
fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source).map_err(|error| files::io(source, error))?;
    if metadata.file_type().is_symlink() {
        let link = fs::read_link(source).map_err(|error| files::io(source, error))?;
        return std::os::unix::fs::symlink(link, target).map_err(|error| files::io(target, error));
    }
    if !metadata.is_dir() {
        return copy(source, target);
    }
    fs::create_dir(target).map_err(|error| files::io(target, error))?;
    for child in fs::read_dir(source).map_err(|error| files::io(source, error))? {
        let child = child.map_err(|error| files::io(source, error))?;
        copy_tree(&child.path(), &target.join(child.file_name()))?;
    }
    Ok(())
}
