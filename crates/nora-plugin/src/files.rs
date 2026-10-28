use nora_plugin_manifest::{Error, Result};
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

pub const JSON_LIMIT: usize = 4 * 1024 * 1024;

pub fn read_limited(path: impl AsRef<Path>, limit: usize, oversized: &str) -> Result<Vec<u8>> {
    let path = path.as_ref();
    let size = fs::metadata(path).map_err(|error| io(path, error))?.len();
    if size > limit as u64 {
        return Err(Error::new(oversized));
    }
    fs::read(path).map_err(|error| io(path, error))
}

pub fn read_json(path: impl AsRef<Path>, limit: usize) -> Result<Value> {
    let bytes = read_limited(&path, limit, "JSON file exceeds size limit")?;
    serde_json::from_slice(&bytes)
        .map_err(|error| Error::new(format!("Read {}: {error}", path.as_ref().display())))
}

pub fn read_text(path: impl AsRef<Path>) -> Result<String> {
    let path = path.as_ref();
    fs::read_to_string(path).map_err(|error| io(path, error))
}

pub fn write(path: impl AsRef<Path>, contents: &[u8]) -> Result<()> {
    let path = path.as_ref();
    fs::write(path, contents).map_err(|error| io(path, error))
}

/// Writes a file that must not exist yet, so nothing an author already has
/// is replaced.
pub fn create(path: impl AsRef<Path>, contents: &[u8]) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| io(parent, error))?;
    }
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut file| file.write_all(contents))
        .map_err(|error| io(path, error))
}

pub fn io(path: &Path, error: std::io::Error) -> Error {
    Error::new(format!("{}: {error}", path.display()))
}
