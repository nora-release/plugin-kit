use crate::error::{ensure, Error, Result};
use crate::icon::{validate_icon_data, MAXIMUM_ICON_BYTES};
use crate::native::{validate_native_manifest, MAXIMUM_MANIFEST_BYTES};
use crate::registry::canonical;
use flate2::read::DeflateDecoder;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use unicode_normalization::UnicodeNormalization;

pub const MAXIMUM_ARCHIVE_BYTES: usize = 256 * 1024 * 1024;
const MAXIMUM_EXPANDED_BYTES: u64 = 512 * 1024 * 1024;
const MAXIMUM_ENTRIES: usize = 20_000;
const MAXIMUM_LINK_BYTES: usize = 4096;
const MANIFEST_PATH: &str = "Contents/Resources/nora-plugin.json";

const END_RECORD: u32 = 0x0605_4b50;
const CENTRAL_RECORD: u32 = 0x0201_4b50;
const LOCAL_RECORD: u32 = 0x0403_4b50;
const TYPE_MASK: u32 = 0o170000;
const TYPE_DIRECTORY: u32 = 0o040000;
const TYPE_FILE: u32 = 0o100000;
const TYPE_LINK: u32 = 0o120000;

struct Stored {
    size: usize,
    method: u16,
    start: usize,
    compressed: usize,
    checksum: u32,
}

/// Reads and checks the manifest of a native plugin release ZIP: one `.app`,
/// no path that leaves it, no link that escapes it, and its icon when named.
///
/// The archive is parsed by hand, so split, ZIP64, and encrypted archives
/// are refused instead of interpreted.
pub fn manifest_from_archive(data: &[u8]) -> Result<Value> {
    ensure(
        data.len() >= 22 && data.len() <= MAXIMUM_ARCHIVE_BYTES,
        "Invalid or oversized ZIP",
    )?;
    let end = (data.len().saturating_sub(65557)..=data.len() - 22)
        .rev()
        .find(|&offset| {
            u32_at(data, offset) == END_RECORD
                && offset + 22 + u16_at(data, offset + 20) == data.len()
        })
        .ok_or_else(|| Error::new("ZIP end record is missing"))?;
    let count = u16_at(data, end + 10);
    let central = u32_at(data, end + 16) as usize;
    ensure(
        count > 0
            && count <= MAXIMUM_ENTRIES
            && u16_at(data, end + 4) == 0
            && u16_at(data, end + 6) == 0
            && u16_at(data, end + 8) == count
            && central + u32_at(data, end + 12) as usize == end,
        "Split, ZIP64, or oversized archives are unsupported",
    )?;
    let mut offset = central;
    let mut expanded = 0u64;
    let mut names = HashSet::new();
    let mut roots = HashSet::new();
    let mut links = HashSet::new();
    let mut paths = Vec::new();
    let mut files = HashMap::new();
    let mut manifest = None;
    for _ in 0..count {
        ensure(
            offset + 46 <= end && u32_at(data, offset) == CENTRAL_RECORD,
            "Invalid ZIP directory",
        )?;
        let flags = u16_value(data, offset + 8);
        let method = u16_value(data, offset + 10);
        let checksum = u32_at(data, offset + 16);
        let compressed = u32_at(data, offset + 20) as usize;
        let size = u32_at(data, offset + 24) as usize;
        let name_length = u16_at(data, offset + 28);
        let local = u32_at(data, offset + 42) as usize;
        let next =
            offset + 46 + name_length + u16_at(data, offset + 30) + u16_at(data, offset + 32);
        ensure(
            name_length > 0
                && name_length <= 4096
                && next <= end
                && flags & 1 == 0
                && matches!(method, 0 | 8)
                && u16_at(data, offset + 34) == 0,
            "Encrypted or malformed ZIP entry",
        )?;
        let name_bytes = &data[offset + 46..offset + 46 + name_length];
        let name =
            std::str::from_utf8(name_bytes).map_err(|_| Error::new("ZIP paths must be UTF-8"))?;
        let parts: Vec<&str> = name.strip_suffix('/').unwrap_or(name).split('/').collect();
        ensure(
            parts
                .iter()
                .all(|part| !part.is_empty() && *part != "." && *part != "..")
                && !name.contains(['\\', '\0']),
            "Unsafe ZIP path",
        )?;
        let normalized = parts.join("/").nfc().collect::<String>().to_lowercase();
        ensure(names.insert(normalized.clone()), "Duplicate ZIP path")?;
        paths.push(normalized.clone());
        if parts[0] != "__MACOSX" {
            roots.insert(parts[0].to_owned());
        }
        expanded += size as u64;
        ensure(
            expanded <= MAXIMUM_EXPANDED_BYTES
                && local + 30 <= central
                && u32_at(data, local) == LOCAL_RECORD
                && u16_value(data, local + 6) == flags
                && u16_value(data, local + 8) == method
                && u16_at(data, local + 26) == name_length,
            "Invalid ZIP local record or size limit",
        )?;
        let start = local + 30 + name_length + u16_at(data, local + 28);
        ensure(
            start + compressed <= central
                && &data[local + 30..local + 30 + name_length] == name_bytes,
            "ZIP entry boundaries do not match",
        )?;
        let kind = (u32_at(data, offset + 38) >> 16) & TYPE_MASK;
        ensure(
            matches!(kind, 0 | TYPE_DIRECTORY | TYPE_FILE | TYPE_LINK),
            "Unsupported ZIP file type",
        )?;
        let stored = Stored {
            size,
            method,
            start,
            compressed,
            checksum,
        };
        if kind == TYPE_LINK {
            ensure(
                size > 0 && size <= MAXIMUM_LINK_BYTES && !name.ends_with('/'),
                "Invalid ZIP symbolic link",
            )?;
            let target = contents(
                data,
                &stored,
                MAXIMUM_LINK_BYTES,
                "ZIP symbolic link checksum does not match",
            )?;
            let target = String::from_utf8(target)
                .map_err(|_| Error::new("Unsafe ZIP symbolic link target"))?;
            ensure(
                !target.starts_with('/') && !target.contains(['\\', '\0']),
                "Unsafe ZIP symbolic link target",
            )?;
            let resolved = normalize(&format!("{}/{target}", parts[..parts.len() - 1].join("/")));
            ensure(
                resolved.starts_with(&format!("{}/", parts[0])),
                "ZIP symbolic link escapes its application",
            )?;
            links.insert(normalized);
        }
        if parts[0] != "__MACOSX" && parts[1..].join("/") == MANIFEST_PATH {
            ensure(
                manifest.is_none()
                    && kind != TYPE_LINK
                    && !name.ends_with('/')
                    && size <= MAXIMUM_MANIFEST_BYTES,
                "Invalid embedded manifest",
            )?;
            let bytes = contents(
                data,
                &stored,
                MAXIMUM_MANIFEST_BYTES,
                "Embedded manifest checksum does not match",
            )?;
            let value: Value = serde_json::from_slice(&bytes)
                .map_err(|_| Error::new("Embedded manifest must be JSON"))?;
            validate_native_manifest(&value)?;
            manifest = Some(value);
        }
        if matches!(kind, 0 | TYPE_FILE) && !name.ends_with('/') {
            files.insert(name.to_owned(), stored);
        }
        offset = next;
    }
    ensure(
        offset == end && roots.len() == 1 && manifest.is_some(),
        "ZIP must contain one plugin folder and its Nora manifest",
    )?;
    let root = roots.into_iter().next().unwrap_or_default();
    ensure(
        root.ends_with(".app"),
        "Native ZIP must contain one .app and its Nora manifest",
    )?;
    for path in &paths {
        let parts: Vec<&str> = path.split('/').collect();
        ensure(
            (1..parts.len()).all(|index| !links.contains(&parts[..index].join("/"))),
            "ZIP entry passes through a symbolic link",
        )?;
    }
    let manifest = manifest.unwrap_or_default();
    if let Some(icon_path) = manifest.get("iconPath").and_then(Value::as_str) {
        let icon = files
            .get(&format!("{root}/Contents/Resources/{icon_path}"))
            .filter(|icon| icon.size > 0 && icon.size <= MAXIMUM_ICON_BYTES)
            .ok_or_else(|| Error::new("ZIP is missing its iconPath or the icon exceeds 512 KiB"))?;
        let bytes = contents(
            data,
            icon,
            MAXIMUM_ICON_BYTES,
            "Plugin icon checksum does not match",
        )?;
        validate_icon_data(&bytes, icon_path)?;
    }
    Ok(manifest)
}

/// Checks that a release archive is the one its marketplace entry describes.
pub fn verify_archive(entry: &Value, archive: &[u8]) -> Result<()> {
    let digest = sha256_hex(archive);
    ensure(
        entry.get("sha256").and_then(Value::as_str) == Some(digest.as_str()),
        "Archive SHA-256 does not match registry",
    )?;
    let manifest = manifest_from_archive(archive)?;
    ensure(
        canonical(&manifest) == canonical(entry.get("manifest").unwrap_or(&Value::Null)),
        "Archive manifest does not match registry",
    )
}

pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn contents(data: &[u8], stored: &Stored, maximum: usize, mismatch: &str) -> Result<Vec<u8>> {
    let raw = &data[stored.start..stored.start + stored.compressed];
    let bytes = if stored.method == 0 {
        raw.to_vec()
    } else {
        let mut bytes = Vec::new();
        DeflateDecoder::new(raw)
            .take(maximum as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::new(mismatch))?;
        ensure(bytes.len() <= maximum, "ZIP entry expands past its limit")?;
        bytes
    };
    ensure(
        bytes.len() == stored.size && crc32fast(&bytes) == stored.checksum,
        mismatch,
    )?;
    Ok(bytes)
}

/// POSIX path normalization of a relative path, keeping leading `..`.
fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." if parts.last().is_some_and(|last| *last != "..") => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("/")
}

fn crc32fast(data: &[u8]) -> u32 {
    let mut hasher = flate2::Crc::new();
    hasher.update(data);
    hasher.sum()
}

fn u16_value(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

fn u16_at(data: &[u8], offset: usize) -> usize {
    u16_value(data, offset) as usize
}

fn u32_at(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}
