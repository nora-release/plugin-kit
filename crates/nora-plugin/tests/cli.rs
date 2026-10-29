use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SDK_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

struct Temporary(PathBuf);

impl Temporary {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("nora-plugin-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn tool(arguments: &[&str]) -> Output {
    tool_with_path(arguments, None)
}

fn tool_with_path(arguments: &[&str], path: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nora-plugin"));
    command.args(arguments);
    if let Some(path) = path {
        let inherited = std::env::var("PATH").unwrap_or_default();
        command.env("PATH", format!("{}:{inherited}", path.display()));
    }
    command.output().unwrap()
}

fn succeeds(output: Output) -> Output {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn text(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path).unwrap()
}

fn path(path: &Path) -> &str {
    path.to_str().unwrap()
}

#[test]
fn registry_scaffold_validates_and_is_not_replaced() {
    let temporary = Temporary::new("registry");
    let repository = temporary.0.join("registry");
    succeeds(tool(&["init-registry", "--directory", path(&repository)]));
    assert!(text(repository.join("NoraPluginKit-LICENSE")).contains("MIT License"));
    succeeds(tool(&[
        "validate",
        "--registry",
        path(&repository.join("registry.json")),
        "--catalog",
        path(&repository.join("marketplace.json")),
        "--verify-downloads",
    ]));
    let workflow = text(repository.join(".github/workflows/validate-marketplace.yml"));
    assert!(workflow.contains("pull_request:") && workflow.contains("pull_request.base.sha"));
    for forbidden in [
        "pull_request_target",
        "secrets.",
        "contents: write",
        "node",
        "python",
        "__PLUGIN_KIT_VERSION__",
    ] {
        assert!(!workflow.contains(forbidden), "{forbidden}");
    }
    assert!(workflow.contains(&format!("--tag v{}", env!("CARGO_PKG_VERSION"))));
    assert!(!tool(&["init-registry", "--directory", path(&repository)])
        .status
        .success());
}

#[test]
fn stale_catalogs_are_refused() {
    let temporary = Temporary::new("stale");
    let registry = temporary.0.join("registry.json");
    let catalog = temporary.0.join("marketplace.json");
    fs::write(
        &registry,
        r#"{"schemaVersion":1,"name":"Tests","plugins":[]}"#,
    )
    .unwrap();
    succeeds(tool(&[
        "generate",
        "--registry",
        path(&registry),
        "--output",
        path(&catalog),
    ]));
    assert_eq!(
        text(&catalog),
        "{\n  \"schemaVersion\": 1,\n  \"name\": \"Tests\",\n  \"plugins\": []\n}\n"
    );
    fs::write(&catalog, "{}").unwrap();
    assert!(!tool(&[
        "validate",
        "--registry",
        path(&registry),
        "--catalog",
        path(&catalog)
    ])
    .status
    .success());
}

#[test]
fn author_scaffold_pins_this_sdk() {
    let temporary = Temporary::new("author");
    let author = temporary.0.join("author");
    succeeds(tool(&["init-author", "--directory", path(&author)]));
    assert!(!tool(&["init-author", "--directory", path(&author)])
        .status
        .success());
    assert!(!tool(&[
        "init-script-author",
        "--directory",
        path(&temporary.0.join("script"))
    ])
    .status
    .success());
    let package = text(author.join("Package.swift"));
    let version = env!("CARGO_PKG_VERSION");
    assert!(package.contains(&format!(
        r#".package(url: "https://github.com/nora-release/plugin-kit.git", exact: "{version}")"#
    )));
    assert!(text(author.join("NoraPluginKit-LICENSE")).contains("MIT License"));
    succeeds(tool(&[
        "validate-manifest",
        "--manifest",
        path(&author.join("nora-plugin.json")),
    ]));
    assert!(
        text(author.join("Sources/PluginExtension/NativePlugin.swift"))
            .contains("import NoraPluginKit")
    );
    let publishing = text(author.join(".github/workflows/release-plugin.yml"));
    for expected in [
        "contents: read",
        "--verify-tag",
        "--draft=false --latest",
        "nora-plugin bundle",
    ] {
        assert!(publishing.contains(expected), "{expected}");
    }
    for forbidden in [
        "pull_request",
        "--clobber",
        "set -x",
        "node",
        "python",
        "__PLUGIN_KIT_VERSION__",
    ] {
        assert!(!publishing.contains(forbidden), "{forbidden}");
    }
    let script = author.join("scripts/publish-native-plugin.sh");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        fs::metadata(&script).unwrap().permissions().mode() & 0o111,
        0o111
    );
    assert!(!text(&script).contains("node ") && !text(&script).contains("python"));

    let local = temporary.0.join("local");
    succeeds(tool(&[
        "init-author",
        "--directory",
        path(&local),
        "--sdk-path",
        SDK_ROOT,
    ]));
    assert!(text(local.join("Package.swift")).contains(r#".package(name: "plugin-kit", path:"#));
    let conflicting = temporary.0.join("conflicting");
    assert!(!tool(&[
        "init-author",
        "--directory",
        path(&conflicting),
        "--sdk-path",
        SDK_ROOT,
        "--sdk-version",
        "1.0.0"
    ])
    .status
    .success());
    let custom = temporary.0.join("custom");
    succeeds(tool(&[
        "init-author",
        "--directory",
        path(&custom),
        "--sdk-url",
        "https://github.com/example/sdk.git",
        "--sdk-version",
        "1.2.3",
    ]));
    let package = text(custom.join("Package.swift"));
    assert!(package.contains(r#"url: "https://github.com/example/sdk.git", exact: "1.2.3""#));
    assert!(package.contains(r#"package: "sdk""#));
}

#[test]
fn sdk_version_matches_the_workspace() {
    succeeds(tool(&[
        "sdk-version",
        "--file",
        &format!("{SDK_ROOT}/sdk-version.json"),
    ]));
    succeeds(tool(&[
        "sdk-version",
        "--tag",
        &format!("v{}", env!("CARGO_PKG_VERSION")),
    ]));
    assert!(!tool(&["sdk-version", "--tag", "v0.0.1"]).status.success());
}

/// Stands in for `swift build` and `lipo`, so packaging is checked without
/// building the SDK.
fn build_tools(directory: &Path, binaries: &Path) -> PathBuf {
    let tools = directory.join("tools");
    fs::create_dir_all(&tools).unwrap();
    let swift = format!(
        "#!/bin/bash\nfor argument in \"$@\"; do [[ \"$argument\" == --show-bin-path ]] && {{ echo {}; exit 0; }}; done\nexit 0\n",
        binaries.display()
    );
    for (name, script) in [
        ("swift", swift.as_str()),
        ("lipo", "#!/bin/bash\necho arm64 x86_64\n"),
    ] {
        let path = tools.join(name);
        fs::write(&path, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    tools
}

fn plist(path: impl AsRef<Path>) -> plist::Dictionary {
    plist::Value::from_file(path)
        .unwrap()
        .into_dictionary()
        .unwrap()
}

#[test]
fn generated_native_scaffold_packages_only_its_containing_app_as_an_agent() {
    let temporary = Temporary::new("bundle");
    let author = temporary.0.join("author");
    succeeds(tool(&[
        "init-author",
        "--directory",
        path(&author),
        "--sdk-path",
        SDK_ROOT,
    ]));
    let binaries = temporary.0.join("binaries");
    fs::create_dir_all(&binaries).unwrap();
    for product in ["PluginHost", "PluginExtension"] {
        fs::write(binaries.join(product), product).unwrap();
    }
    let tools = build_tools(&temporary.0, &binaries);
    let dist = temporary.0.join("dist");
    let config = author.join("plugin-release.json");
    let arguments = [
        "bundle",
        "--config",
        path(&config),
        "--output",
        path(&dist),
        "--tag",
        "v0.1.0",
    ];
    succeeds(tool_with_path(&arguments, Some(&tools)));
    assert!(
        !tool_with_path(&arguments, Some(&tools)).status.success(),
        "an existing app must not be replaced"
    );
    assert!(!tool_with_path(
        &[
            "bundle",
            "--config",
            path(&config),
            "--output",
            path(&temporary.0.join("other")),
            "--tag",
            "v9.9.9"
        ],
        Some(&tools)
    )
    .status
    .success());

    let app = dist.join("com.example.nora.hello.app");
    let extension = app.join("Contents/Extensions/NativePlugin.appex");
    let host = plist(app.join("Contents/Info.plist"));
    let extension_info = plist(extension.join("Contents/Info.plist"));
    assert_eq!(
        host.get("LSUIElement").and_then(plist::Value::as_boolean),
        Some(true)
    );
    assert_eq!(
        host.get("CFBundlePackageType")
            .and_then(plist::Value::as_string),
        Some("APPL")
    );
    assert!(!host.contains_key("LSBackgroundOnly"));
    assert!(
        !extension_info.contains_key("LSUIElement")
            && !extension_info.contains_key("LSBackgroundOnly")
    );
    assert_eq!(
        extension_info
            .get("CFBundlePackageType")
            .and_then(plist::Value::as_string),
        Some("XPC!")
    );
    let attributes = extension_info
        .get("EXAppExtensionAttributes")
        .and_then(plist::Value::as_dictionary)
        .unwrap();
    assert_eq!(
        attributes
            .get("EXExtensionPointIdentifier")
            .and_then(plist::Value::as_string),
        Some("com.nora.plugin.native")
    );
    assert_eq!(
        fs::read(app.join("Contents/MacOS/PluginHost")).unwrap(),
        b"PluginHost"
    );
    assert_eq!(
        fs::read(extension.join("Contents/MacOS/PluginExtension")).unwrap(),
        b"PluginExtension"
    );
    let packaged: serde_json::Value =
        serde_json::from_str(&text(app.join("Contents/Resources/nora-plugin.json"))).unwrap();
    let source: serde_json::Value =
        serde_json::from_str(&text(author.join("nora-plugin.json"))).unwrap();
    assert_eq!(packaged, source);
}

#[cfg(target_os = "macos")]
#[test]
fn generated_containing_host_typechecks_in_swift_6_without_sdk_dependencies() {
    let temporary = Temporary::new("typecheck");
    let author = temporary.0.join("author");
    succeeds(tool(&[
        "init-author",
        "--directory",
        path(&author),
        "--sdk-path",
        SDK_ROOT,
    ]));
    let host = author.join("Sources/PluginHost/PluginHost.swift");
    let output = Command::new("xcrun")
        .args([
            "swiftc",
            "-typecheck",
            "-parse-as-library",
            "-swift-version",
            "6",
            path(&host),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// After `make example`, checks that the example and a generated scaffold
/// register without a Dock icon or a window, and still show a window when
/// opened by hand. Run with `make registration`.
#[cfg(target_os = "macos")]
#[test]
#[ignore = "needs make example and a logged-in macOS session"]
fn containing_apps_register_quietly() {
    let example = fs::canonicalize(format!("{SDK_ROOT}/.build/NoraPluginExample.app"))
        .expect("run make example first");
    let temporary = Temporary::new("registration");
    let probe = temporary.0.join("native-host-probe");
    let compile = |source: &Path, output: &Path| {
        let result = Command::new("xcrun")
            .args([
                "swiftc",
                "-parse-as-library",
                "-swift-version",
                "6",
                path(source),
                "-o",
                path(output),
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    };
    compile(
        Path::new(&format!("{SDK_ROOT}/scripts/native-host-probe.swift")),
        &probe,
    );
    let author = temporary.0.join("author");
    succeeds(tool(&[
        "init-author",
        "--directory",
        path(&author),
        "--sdk-path",
        SDK_ROOT,
    ]));
    let binaries = temporary.0.join("binaries");
    fs::create_dir_all(&binaries).unwrap();
    for product in ["PluginHost", "PluginExtension"] {
        fs::write(binaries.join(product), product).unwrap();
    }
    let tools = build_tools(&temporary.0, &binaries);
    let dist = temporary.0.join("dist");
    let config = author.join("plugin-release.json");
    succeeds(tool_with_path(
        &[
            "bundle",
            "--config",
            path(&config),
            "--output",
            path(&dist),
            "--tag",
            "v0.1.0",
        ],
        Some(&tools),
    ));
    let generated = dist.join("com.example.nora.hello.app");
    compile(
        &author.join("Sources/PluginHost/PluginHost.swift"),
        &generated.join("Contents/MacOS/PluginHost"),
    );
    for app in [example, generated] {
        let info = plist(app.join("Contents/Info.plist"));
        let executable = app
            .join("Contents/MacOS")
            .join(info["CFBundleExecutable"].as_string().unwrap());
        for mode in ["register", "workspace", "preview"] {
            let output = Command::new(&probe)
                .args([path(&executable), mode])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{} {mode}: {}",
                app.display(),
                String::from_utf8_lossy(&output.stderr)
            );
            let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert!(report["elapsedSeconds"].as_f64().unwrap() < 3.0, "{report}");
            assert!(
                report["violations"].as_array().unwrap().is_empty(),
                "{report}"
            );
            match mode {
                "register" => assert!(
                    report["exitStatus"] == 0 && report["visibleWindowObserved"] == false,
                    "{report}"
                ),
                "preview" => assert!(
                    report["visibleWindowObserved"] == true
                        && report["accessoryPreviewObserved"] == true,
                    "{report}"
                ),
                _ => assert!(
                    report["workspaceCompleted"] == true
                        && report["workspaceReturnedApplication"] == true,
                    "{report}"
                ),
            }
        }
    }
}

#[cfg(target_os = "macos")]
#[test]
fn release_entry_describes_a_ditto_archive() {
    let temporary = Temporary::new("release");
    let resources = temporary.0.join("Test.app/Contents/Resources");
    fs::create_dir_all(&resources).unwrap();
    let manifest = fs::read_to_string(format!("{SDK_ROOT}/Examples/nora-plugin.json")).unwrap();
    fs::write(resources.join("nora-plugin.json"), &manifest).unwrap();
    let manifest_path = temporary.0.join("nora-plugin.json");
    fs::write(&manifest_path, &manifest).unwrap();
    let archive = temporary.0.join("plugin.zip");
    let zipped = Command::new("ditto")
        .args([
            "-c",
            "-k",
            "--keepParent",
            path(&temporary.0.join("Test.app")),
            path(&archive),
        ])
        .status()
        .unwrap();
    assert!(zipped.success());
    let output = temporary.0.join("release");
    let arguments = [
        "release-entry",
        "--manifest",
        path(&manifest_path),
        "--archive",
        path(&archive),
        "--repository",
        "https://github.com/nora-tests/plugin",
        "--tag",
        "v1.0.0",
        "--team",
        "ABCDE12345",
        "--output",
        path(&output),
    ];
    succeeds(tool(&arguments));
    let marketplace: serde_json::Value =
        serde_json::from_str(&text(output.join("marketplace.json"))).unwrap();
    let entry = &marketplace["plugins"][0];
    assert_eq!(
        entry["downloadURL"],
        "https://github.com/nora-tests/plugin/releases/download/v1.0.0/plugin.zip"
    );
    assert_eq!(
        text(output.join("SHA256SUMS")),
        format!("{}  plugin.zip\n", entry["sha256"].as_str().unwrap())
    );
    assert!(
        !tool(&arguments).status.success(),
        "release metadata must not be replaced"
    );
}
