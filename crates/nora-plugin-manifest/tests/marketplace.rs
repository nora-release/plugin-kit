use flate2::write::DeflateEncoder;
use flate2::Compression;
use nora_plugin_manifest::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::Write;

pub struct ZipEntry<'a> {
    pub name: &'a str,
    pub data: Vec<u8>,
    pub stored: bool,
    pub mode: u32,
}

pub fn file(name: &str, data: impl Into<Vec<u8>>) -> ZipEntry<'_> {
    ZipEntry {
        name,
        data: data.into(),
        stored: false,
        mode: 0o100644,
    }
}

pub fn link<'a>(name: &'a str, target: &str) -> ZipEntry<'a> {
    ZipEntry {
        name,
        data: target.as_bytes().to_vec(),
        stored: false,
        mode: 0o120777,
    }
}

fn crc(data: &[u8]) -> u32 {
    let mut crc = flate2::Crc::new();
    crc.update(data);
    crc.sum()
}

pub fn zip(entries: &[ZipEntry<'_>]) -> Vec<u8> {
    let (mut locals, mut centrals) = (Vec::new(), Vec::new());
    for entry in entries {
        let name = entry.name.as_bytes();
        let compressed = if entry.stored {
            entry.data.clone()
        } else {
            let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(&entry.data).unwrap();
            encoder.finish().unwrap()
        };
        let method: u16 = if entry.stored { 0 } else { 8 };
        let offset = locals.len() as u32;
        locals.extend(0x0403_4b50u32.to_le_bytes());
        locals.extend(20u16.to_le_bytes());
        locals.extend(0u16.to_le_bytes());
        locals.extend(method.to_le_bytes());
        locals.extend([0; 4]);
        locals.extend(crc(&entry.data).to_le_bytes());
        locals.extend((compressed.len() as u32).to_le_bytes());
        locals.extend((entry.data.len() as u32).to_le_bytes());
        locals.extend((name.len() as u16).to_le_bytes());
        locals.extend(0u16.to_le_bytes());
        locals.extend(name);
        locals.extend(&compressed);
        centrals.extend(0x0201_4b50u32.to_le_bytes());
        centrals.extend(20u16.to_le_bytes());
        centrals.extend(20u16.to_le_bytes());
        centrals.extend(0u16.to_le_bytes());
        centrals.extend(method.to_le_bytes());
        centrals.extend([0; 4]);
        centrals.extend(crc(&entry.data).to_le_bytes());
        centrals.extend((compressed.len() as u32).to_le_bytes());
        centrals.extend((entry.data.len() as u32).to_le_bytes());
        centrals.extend((name.len() as u16).to_le_bytes());
        centrals.extend([0; 8]);
        centrals.extend((entry.mode << 16).to_le_bytes());
        centrals.extend(offset.to_le_bytes());
        centrals.extend(name);
    }
    let directory_offset = locals.len() as u32;
    let mut archive = locals;
    let directory_length = centrals.len() as u32;
    archive.extend(centrals);
    archive.extend(0x0605_4b50u32.to_le_bytes());
    archive.extend([0; 4]);
    archive.extend((entries.len() as u16).to_le_bytes());
    archive.extend((entries.len() as u16).to_le_bytes());
    archive.extend(directory_length.to_le_bytes());
    archive.extend(directory_offset.to_le_bytes());
    archive.extend(0u16.to_le_bytes());
    archive
}

fn manifest() -> Value {
    json!({
        "schemaVersion": 1, "apiVersion": 1, "id": "com.nora.test.native", "name": "Test Plugin", "version": "1.2.3",
        "summary": "A test fixture", "publisher": "Nora Tests", "extensionBundleIdentifier": "com.nora.test.native.extension",
        "permissions": ["hostInfo"], "panels": [{ "id": "com.nora.test.native.panel", "title": "Test", "systemImage": "puzzlepiece" }],
        "commands": [{ "id": "com.nora.test.native.echo", "title": "Echo", "description": "Echo values", "inputSchemaJSON": "{\"type\":\"object\"}", "readOnly": true }],
    })
}

fn with(mut value: Value, patch: Value) -> Value {
    for (key, field) in patch.as_object().unwrap() {
        value[key] = field.clone();
    }
    value
}

fn without(mut value: Value, key: &str) -> Value {
    value.as_object_mut().unwrap().remove(key);
    value
}

const MANIFEST_NAME: &str = "Test.app/Contents/Resources/nora-plugin.json";
const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M0 0h24v24H0z"/></svg>"#;

fn archive() -> Vec<u8> {
    zip(&[file(MANIFEST_NAME, manifest().to_string())])
}

fn entry() -> Value {
    json!({
        "manifest": manifest(),
        "repositoryURL": "https://github.com/nora-tests/plugin", "releaseTag": "v1.2.3", "teamIdentifier": "ABCDE12345",
        "downloadURL": "https://github.com/nora-tests/plugin/releases/download/v1.2.3/plugin.zip",
        "sha256": format!("{:x}", Sha256::digest(archive())),
    })
}

#[test]
fn manifests_follow_the_native_contract() {
    assert_eq!(validate_native_manifest(&manifest()).unwrap(), &manifest());
    for version in ["1.2", "01.2.3", "1.2.3-01", "1.2.3-", "latest"] {
        assert!(
            validate_native_manifest(&with(manifest(), json!({ "version": version }))).is_err(),
            "{version}"
        );
    }
    for version in ["1.2.3", "1.2.3-beta.1", "1.2.3+build.2"] {
        validate_native_manifest(&with(manifest(), json!({ "version": version }))).unwrap();
    }
    let panel_id = manifest()["panels"][0]["id"].clone();
    let mut command = manifest()["commands"][0].clone();
    command["id"] = panel_id;
    let mut array_schema = manifest()["commands"][0].clone();
    array_schema["inputSchemaJSON"] = json!("{\"type\":\"array\"}");
    let mut foreign_panel = manifest()["panels"][0].clone();
    foreign_panel["id"] = json!("com.other.panel");
    for patch in [
        json!({ "permissions": ["filesystem"] }),
        json!({ "permissions": ["chat", "chat"] }),
        json!({ "commands": [command] }),
        json!({ "commands": [array_schema] }),
        json!({ "panels": [foreign_panel] }),
        json!({ "runtime": "script" }),
        json!({ "runtime": "nativeExtension" }),
        json!({ "runtime": null }),
        json!({ "entryPoint": "plugin.js" }),
        json!({ "entryPoint": null }),
        json!({ "schemaVersion": 2 }),
    ] {
        assert!(
            validate_native_manifest(&with(manifest(), patch.clone())).is_err(),
            "{patch}"
        );
    }
}

#[test]
fn network_domains_are_exact_hosts_behind_the_network_permission() {
    let network = with(
        manifest(),
        json!({
            "networkDomains": ["api.github.com", "github.com"],
            "permissions": ["hostInfo", "notifications", "network", "credentials", "openExternal", "chat"],
        }),
    );
    validate_native_manifest(&network).unwrap();
    for domains in [
        json!(["*.github.com"]),
        json!(["https://github.com"]),
        json!(["GitHub.com"]),
        json!(["github.com:443"]),
        json!(["github.com\n"]),
        json!(["github.com", "github.com"]),
    ] {
        assert!(validate_native_manifest(&with(
            network.clone(),
            json!({ "networkDomains": domains })
        ))
        .is_err());
    }
    assert!(validate_native_manifest(&with(network, json!({ "permissions": [] }))).is_err());
}

#[test]
fn icon_paths_stay_inside_resources() {
    for icon in [
        "assets/brand.svg",
        "brand.png",
        "images/brand.jpg",
        "images/brand.jpeg",
    ] {
        validate_native_manifest(&with(manifest(), json!({ "iconPath": icon }))).unwrap();
    }
    let deep = format!("{}brand.svg", "a/".repeat(513));
    for icon in [
        json!(""),
        json!("../brand.svg"),
        json!("/brand.svg"),
        json!("a/../brand.svg"),
        json!("a/./brand.svg"),
        json!("a//brand.svg"),
        json!("a\\brand.svg"),
        json!("brand.svg\n"),
        json!("brand.js"),
        json!("brand.SVG"),
        json!("Contents/Resources/brand.svg"),
        json!("contents/resources/brand.svg"),
        json!(deep),
        json!(null),
    ] {
        assert!(
            validate_native_manifest(&with(manifest(), json!({ "iconPath": icon.clone() })))
                .is_err(),
            "{icon}"
        );
    }
}

#[test]
fn icons_are_static_images() {
    validate_icon_data(SVG.as_bytes(), "assets/brand.svg").unwrap();
    for icon in [
        r#"<!DOCTYPE svg [<!ENTITY x SYSTEM "file:///tmp/private">]><svg>&x;</svg>"#,
        "<svg><script>alert(1)</script></svg>",
        r#"<svg><image href="https://example.com/image.png"/></svg>"#,
        r##"<svg><use href="#linked"/></svg>"##,
        "<svg><foreignObject/></svg>",
        r#"<svg><path onclick="run()"/></svg>"#,
        r#"<svg><path style="fill:u&#114;l(https://example.com/x)"/></svg>"#,
        r#"<svg><animate attributeName="href"/></svg>"#,
        r#"<svg><path fill="u&#114;l(https://example.com/x)"/></svg>"#,
        "<svg><path></svg>",
        "<svg>&undefined;</svg>",
        "<html/>",
    ] {
        assert!(
            validate_icon_data(icon.as_bytes(), "brand.svg").is_err(),
            "{icon}"
        );
    }
    assert!(validate_icon_data(&vec![0; MAXIMUM_ICON_BYTES + 1], "brand.png").is_err());
    assert!(validate_icon_data(b"not PNG", "brand.png").is_err());
}

#[test]
fn archives_hold_one_app_and_its_manifest() {
    assert_eq!(manifest_from_archive(&archive()).unwrap(), manifest());
    let with_icon = with(manifest(), json!({ "iconPath": "brand.svg" }));
    let manifest_file = || file(MANIFEST_NAME, with_icon.to_string());
    assert_eq!(
        manifest_from_archive(&zip(&[
            manifest_file(),
            file("Test.app/Contents/Resources/brand.svg", SVG)
        ]))
        .unwrap(),
        with_icon
    );
    let icon_name = "Test.app/Contents/Resources/brand.svg";
    for icon in [
        None,
        Some(ZipEntry {
            mode: 0o120777,
            ..file(icon_name, SVG)
        }),
        Some(file(icon_name, vec![b' '; MAXIMUM_ICON_BYTES + 1])),
        Some(file(icon_name, "<svg><script/></svg>")),
    ] {
        let mut entries = vec![manifest_file()];
        entries.extend(icon);
        assert!(manifest_from_archive(&zip(&entries)).is_err());
    }
    for name in [
        "TestPlugin/nora-plugin.json",
        "Test.app/nora-plugin.json",
        "TestPlugin/Contents/Resources/nora-plugin.json",
        "../Test.app/Contents/Resources/nora-plugin.json",
        "/Test.app/Contents/Resources/nora-plugin.json",
        "Test.app/../Contents/Resources/nora-plugin.json",
    ] {
        assert!(
            manifest_from_archive(&zip(&[file(name, manifest().to_string())])).is_err(),
            "{name}"
        );
    }
    assert!(manifest_from_archive(&zip(&[file(MANIFEST_NAME, vec![b' '; 513 * 1024])])).is_err());
}

#[test]
fn archive_links_stay_inside_the_app() {
    for target in ["/tmp/outside", "../../../outside", "other\\file"] {
        let entries = [
            file(MANIFEST_NAME, manifest().to_string()),
            link("Test.app/Contents/Resources/link", target),
        ];
        assert!(manifest_from_archive(&zip(&entries)).is_err(), "{target}");
    }
    let entries = [
        file(MANIFEST_NAME, manifest().to_string()),
        link("Test.app/Contents/Link", "Resources"),
        file("Test.app/Contents/link/hidden", "payload"),
    ];
    assert!(manifest_from_archive(&zip(&entries)).is_err());
}

#[test]
fn entries_point_at_their_own_release() {
    assert_eq!(validate_entry(&entry()).unwrap(), entry());
    assert!(validate_entry(&without(entry(), "teamIdentifier")).is_err());
    let unprefixed = with(
        entry(),
        json!({
            "releaseTag": "1.2.3",
            "downloadURL": "https://github.com/nora-tests/plugin/releases/download/1.2.3/plugin.zip",
        }),
    );
    assert_eq!(validate_entry(&unprefixed).unwrap()["releaseTag"], "1.2.3");
    for patch in [
        json!({ "releaseTag": "latest" }),
        json!({ "teamIdentifier": "-" }),
        json!({ "sha256": "0".repeat(64) }),
        json!({ "downloadURL": "https://github.com/other/plugin/releases/download/v1.2.3/plugin.zip" }),
        json!({ "downloadURL": "https://github.com/nora-tests/plugin/releases/latest/download/plugin.zip" }),
        json!({ "downloadURL": "http://github.com/nora-tests/plugin/releases/download/v1.2.3/plugin.zip" }),
        json!({ "downloadURL": "https://github.com/nora-tests/plugin/releases/download/v1.2.3/%2Fbad.zip" }),
        json!({ "repositoryURL": "https://user:password@github.com/nora-tests/plugin" }),
    ] {
        assert!(
            validate_entry(&with(entry(), patch.clone())).is_err(),
            "{patch}"
        );
    }
}

#[test]
fn registries_are_sorted_and_unique() {
    assert!(validate_registry(
        &json!({ "schemaVersion": 1, "name": "Tests", "plugins": [entry(), entry()] })
    )
    .is_err());
    let second = with(
        entry(),
        json!({ "manifest": with(manifest(), json!({ "id": "com.nora.a" })) }),
    );
    let mut second_manifest = second["manifest"].clone();
    second_manifest["extensionBundleIdentifier"] = json!("com.nora.a.extension");
    second_manifest["panels"] = json!([]);
    second_manifest["commands"] = json!([]);
    let second = with(second, json!({ "manifest": second_manifest }));
    let registry = validate_registry(
        &json!({ "schemaVersion": 1, "name": "Tests", "plugins": [entry(), second] }),
    )
    .unwrap();
    assert_eq!(registry["plugins"][0]["manifest"]["id"], "com.nora.a");
    assert!(format_registry(&registry).ends_with("}\n"));
}

#[test]
fn archives_match_their_entry() {
    verify_archive(&entry(), &archive()).unwrap();
    assert!(verify_archive(
        &with(entry(), json!({ "sha256": "1".repeat(64) })),
        &archive()
    )
    .is_err());
    let changed = with(
        entry(),
        json!({ "manifest": with(manifest(), json!({ "permissions": [] })) }),
    );
    assert!(verify_archive(&changed, &archive()).is_err());
}

#[test]
fn the_example_manifest_is_valid() {
    let example: Value =
        serde_json::from_str(include_str!("../../../Examples/nora-plugin.json")).unwrap();
    validate_native_manifest(&example).unwrap();
}
