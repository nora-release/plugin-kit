use nora_plugin_manifest::PluginManifest;
use serde_json::{json, Value};

fn provider_package() -> Value {
    json!({
        "apiVersion": 2, "id": "echo-example", "name": "Echo Example", "version": "0.1.0",
        "runtime": { "entry": "bin/provider" },
        "contributes": {
            "provider": { "api": "openai-completions", "baseURL": "https://echo.invalid/v1", "models": [{ "id": "echo" }] },
            "panels": [{ "slot": "provider", "title": "About", "titles": { "zh-Hans": "关于" }, "file": "ui/panel.html" }],
            "page": { "title": "Echo", "file": "ui/page.html" },
            "actions": [{ "id": "inspect", "menu": "provider", "title": "Inspect" }]
        },
        "reads": ["providers:list"]
    })
}

fn with(mut value: Value, pointer: &str, field: Value) -> Value {
    let (parent, key) = pointer.rsplit_once('/').unwrap();
    value
        .pointer_mut(parent)
        .unwrap()
        .as_object_mut()
        .unwrap()
        .insert(key.to_owned(), field);
    value
}

fn without(mut value: Value, parent: &str, key: &str) -> Value {
    value
        .pointer_mut(parent)
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove(key);
    value
}

fn parse(value: &Value) -> Result<PluginManifest, String> {
    PluginManifest::parse(&serde_json::to_vec(value).unwrap()).map_err(|error| error.to_string())
}

#[test]
fn a_package_names_everything_it_contributes() {
    let manifest = parse(&provider_package()).unwrap();
    let provider = manifest.contributes.provider.as_ref().unwrap();
    assert_eq!(provider.base_url, "https://echo.invalid/v1");
    assert_eq!(manifest.contributes.actions[0].id, "inspect");
    assert_eq!(serde_json::to_value(&manifest).unwrap(), provider_package());
    let middleware = json!({
        "apiVersion": 2, "id": "block-patterns", "name": "Block patterns", "version": "1.2.0",
        "runtime": { "entry": "provider" }, "contributes": { "middleware": { "version": 1 } }
    });
    parse(&middleware).unwrap();
    let page_only = json!({
        "apiVersion": 2, "id": "notes", "name": "Notes", "version": "1.0.0",
        "contributes": { "page": { "title": "Notes", "file": "index.html" } }
    });
    parse(&page_only).unwrap();
}

#[test]
fn old_and_ambiguous_manifests_are_refused() {
    let package = provider_package();
    for refused in [
        with(package.clone(), "/apiVersion", json!(1)),
        with(package.clone(), "/id", json!("Echo")),
        with(package.clone(), "/id", json!("-echo")),
        with(package.clone(), "/version", json!("1.0")),
        with(package.clone(), "/contributes/provider/api", json!("")),
        with(package.clone(), "/runtime/entry", json!("provider.js")),
        with(package.clone(), "/runtime/entry", json!("../provider")),
        with(package.clone(), "/reads", json!(["accounts:secrets"])),
        with(package.clone(), "/contributes/panels/0/slot", json!("chat")),
        with(
            package.clone(),
            "/contributes/panels/0/file",
            json!("ui/panel.js"),
        ),
        with(
            package.clone(),
            "/contributes/actions/0/id",
            json!("Inspect"),
        ),
        with(package.clone(), "/contributes/hooks", json!(true)),
        with(
            package.clone(),
            "/contributes/tools",
            json!({ "version": 1 }),
        ),
        without(package.clone(), "", "runtime"),
        without(package.clone(), "/contributes", "page"),
        with(package.clone(), "/contributes", json!({})),
        with(
            package.clone(),
            "/contributes/provider/aliases",
            json!(["echo"]),
        ),
        with(package.clone(), "/api", json!("openai-completions")),
        json!({ "apiVersion": 2, "id": "notes", "name": "Notes", "version": "1.0.0", "runtime": { "entry": "bin/x" },
                "contributes": { "page": { "title": "Notes", "file": "index.html" } } }),
        json!({ "apiVersion": 2, "id": "notes", "name": "Notes", "version": "1.0.0", "reads": ["providers:list"],
                "runtime": { "entry": "bin/x" }, "contributes": { "middleware": {} } }),
    ] {
        assert!(parse(&refused).is_err(), "{refused}");
    }
}

#[test]
fn named_files_must_stay_inside_the_installed_package() {
    let root = std::env::temp_dir().join(format!("nora-v2-files-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("package/ui")).unwrap();
    std::fs::create_dir_all(root.join("package/bin")).unwrap();
    std::fs::write(root.join("package/bin/provider"), "binary").unwrap();
    std::fs::write(root.join("package/ui/panel.html"), "<p>panel</p>").unwrap();
    std::fs::write(root.join("outside.html"), "<p>outside</p>").unwrap();
    std::os::unix::fs::symlink(root.join("outside.html"), root.join("package/ui/page.html"))
        .unwrap();
    let manifest = parse(&provider_package()).unwrap();
    assert!(manifest.validate_files(&root.join("package")).is_err());
    std::fs::remove_file(root.join("package/ui/page.html")).unwrap();
    std::fs::write(root.join("package/ui/page.html"), "<p>page</p>").unwrap();
    manifest.validate_files(&root.join("package")).unwrap();
    std::fs::remove_file(root.join("package/bin/provider")).unwrap();
    assert!(manifest.validate_files(&root.join("package")).is_err());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_provider_example_is_a_valid_package() {
    let manifest = PluginManifest::parse(include_bytes!(
        "../../../Examples/ProviderPlugin/package/nora-plugin.json"
    ))
    .unwrap();
    assert!(manifest.contributes.provider.is_some());
    assert_eq!(manifest.reads, ["providers:list"]);
    PluginManifest::parse(include_bytes!(
        "../../nora-provider-plugin-sdk/examples/openai-compatible/nora-plugin.json"
    ))
    .unwrap();
}
