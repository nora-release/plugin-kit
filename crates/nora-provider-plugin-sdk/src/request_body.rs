use crate::{PluginError, PluginResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub const MAX_BODY_FILE_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequestBodyFile {
    pub path: PathBuf,
    pub length: u64,
    pub sha256: String,
}

impl RequestBodyFile {
    pub fn from_path(path: impl AsRef<Path>) -> PluginResult<Self> {
        let path = path.as_ref();
        let mut file = open(path)?;
        let (length, sha256) = digest(&mut file)?;
        Ok(Self {
            path: path.into(),
            length,
            sha256,
        })
    }

    /// Open the same regular file that was verified, without following a symlink.
    pub fn open_verified(&self) -> PluginResult<File> {
        if self.length > MAX_BODY_FILE_BYTES
            || self.sha256.len() != 64
            || !self.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(invalid());
        }
        let mut file = open(&self.path)?;
        if file.metadata().map_err(|_| invalid())?.len() != self.length {
            return Err(invalid());
        }
        let (length, digest) = digest(&mut file)?;
        if length != self.length || !digest.eq_ignore_ascii_case(&self.sha256) {
            return Err(invalid());
        }
        file.seek(SeekFrom::Start(0)).map_err(|_| invalid())?;
        Ok(file)
    }

    pub fn read_verified(&self) -> PluginResult<Vec<u8>> {
        let mut file = open(&self.path)?;
        if self.length > MAX_BODY_FILE_BYTES
            || file.metadata().map_err(|_| invalid())?.len() != self.length
        {
            return Err(invalid());
        }
        let mut body = Vec::with_capacity(self.length.try_into().map_err(|_| invalid())?);
        file.by_ref()
            .take(self.length.saturating_add(1))
            .read_to_end(&mut body)
            .map_err(|_| invalid())?;
        if body.len() as u64 != self.length
            || !format!("{:x}", Sha256::digest(&body)).eq_ignore_ascii_case(&self.sha256)
        {
            return Err(invalid());
        }
        Ok(body)
    }

    /// Verify outside the async worker, then stream the file without a second body copy.
    pub async fn http_body(&self) -> PluginResult<reqwest::Body> {
        use tokio::io::AsyncReadExt;
        let reference = self.clone();
        let file = tokio::task::spawn_blocking(move || reference.open_verified())
            .await
            .map_err(|_| invalid())??;
        let reader = tokio::fs::File::from_std(file).take(self.length);
        Ok(reqwest::Body::wrap_stream(
            tokio_util::io::ReaderStream::new(reader),
        ))
    }
}

fn open(path: &Path) -> PluginResult<File> {
    if !path.is_absolute() {
        return Err(invalid());
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|_| invalid())?;
    if !metadata.is_file() || metadata.len() > MAX_BODY_FILE_BYTES {
        return Err(invalid());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|_| invalid())?;
    let metadata = file.metadata().map_err(|_| invalid())?;
    if !metadata.is_file() || metadata.len() > MAX_BODY_FILE_BYTES {
        return Err(invalid());
    }
    Ok(file)
}

fn digest(file: &mut File) -> PluginResult<(u64, String)> {
    let mut digest = Sha256::new();
    let mut length = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let size = file.read(&mut buffer).map_err(|_| invalid())?;
        if size == 0 {
            break;
        }
        length = length.saturating_add(size as u64);
        if length > MAX_BODY_FILE_BYTES {
            return Err(invalid());
        }
        digest.update(&buffer[..size]);
    }
    Ok((length, format!("{:x}", digest.finalize())))
}

fn invalid() -> PluginError {
    PluginError::invalid(
        "The provider request body file is missing, unsafe, changed, or too large.",
    )
}

#[cfg(test)]
#[path = "../test/request_body.test.rs"]
mod tests;
