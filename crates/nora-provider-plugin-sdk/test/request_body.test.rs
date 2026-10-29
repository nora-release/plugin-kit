use super::*;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "nora-body-file-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn file(&self) -> PathBuf {
        self.0.join("body")
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn body_file_preserves_binary_bytes_and_rejects_changes() {
    let directory = Directory::new();
    let bytes = [0, 255, b'\r', b'\n', 0, 127];
    std::fs::write(directory.file(), bytes).unwrap();
    let reference = RequestBodyFile::from_path(directory.file()).unwrap();
    assert_eq!(reference.read_verified().unwrap(), bytes);
    let encoded = serde_json::to_value(&reference).unwrap();
    assert!(encoded.get("body").is_none());
    let decoded: RequestBodyFile = serde_json::from_value(encoded).unwrap();
    assert_eq!(
        decoded.open_verified().unwrap().metadata().unwrap().len(),
        6
    );
    std::fs::write(directory.file(), [0, 0, 0, 0, 0, 0]).unwrap();
    assert!(decoded.read_verified().is_err());
    assert!(decoded.open_verified().is_err());
    std::fs::write(directory.file(), bytes).unwrap();
    OpenOptions::new()
        .append(true)
        .open(directory.file())
        .unwrap()
        .write_all(b"extra")
        .unwrap();
    assert!(decoded.open_verified().is_err());
}

#[test]
fn unsafe_paths_and_oversized_files_fail_without_reading_their_contents() {
    let directory = Directory::new();
    assert!(RequestBodyFile::from_path("relative-body").is_err());
    assert!(RequestBodyFile::from_path(&directory.0).is_err());
    assert!(RequestBodyFile::from_path(directory.file()).is_err());
    let file = File::create(directory.file()).unwrap();
    file.set_len(MAX_BODY_FILE_BYTES + 1).unwrap();
    assert!(RequestBodyFile::from_path(directory.file()).is_err());
    #[cfg(unix)]
    {
        let link = directory.0.join("symlink");
        std::os::unix::fs::symlink(directory.file(), &link).unwrap();
        assert!(RequestBodyFile::from_path(link).is_err());
    }
}

#[tokio::test]
async fn large_request_files_stream_without_base64_protocol_frames() {
    use http_body_util::BodyExt;
    let directory = Directory::new();
    let bytes = vec![0xa5; 33 * 1024 * 1024];
    std::fs::write(directory.file(), &bytes).unwrap();
    let reference = RequestBodyFile::from_path(directory.file()).unwrap();
    let mut body = reference.http_body().await.unwrap();
    assert!(
        body.as_bytes().is_none(),
        "A large upload must remain streaming"
    );
    // The serialized transport stays bounded regardless of the file size.
    let frame = serde_json::to_vec(&reference).unwrap();
    assert!(frame.len() < 4096);
    let mut actual = Vec::new();
    let mut chunks = 0;
    while let Some(frame) = body.frame().await {
        let frame = frame.unwrap();
        if let Some(data) = frame.data_ref() {
            chunks += 1;
            actual.extend_from_slice(data);
        }
    }
    assert!(chunks > 1);
    assert_eq!(actual, bytes);
}

#[test]
fn a_fetch_cannot_mix_inline_and_file_bodies() {
    let request: crate::FetchRequest = serde_json::from_value(serde_json::json!({
        "url":"https://example.invalid","method":"POST","body":"aGk=",
        "bodyFile":{"path":"/private/synthetic-body","length":2,"sha256":"0".repeat(64)}
    }))
    .unwrap();
    assert!(request.decode_body().is_err());
}
