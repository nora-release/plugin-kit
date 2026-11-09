//! Bounded JSONL framing shared by the host and SDK.

use serde_json::Value;
use std::io;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

pub async fn read_frame<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    max_bytes: usize,
) -> io::Result<Option<Value>> {
    let mut bytes = Vec::new();
    read_frame_buffered(reader, &mut bytes, max_bytes).await
}

/// Retains consumed bytes when a caller cancels and resumes a frame read.
pub async fn read_frame_buffered<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    bytes: &mut Vec<u8>,
    max_bytes: usize,
) -> io::Result<Option<Value>> {
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return if bytes.is_empty() {
                Ok(None)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Incomplete plugin frame",
                ))
            };
        }
        let count = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        if bytes.len().saturating_add(count) > max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Plugin frame exceeds the limit",
            ));
        }
        let complete = available[count - 1] == b'\n';
        bytes.extend_from_slice(&available[..count]);
        reader.consume(count);
        if complete {
            let result = serde_json::from_slice(bytes).map(Some).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "Invalid plugin JSON frame")
            });
            bytes.clear();
            return result;
        }
    }
}

pub async fn write_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    value: &Value,
    max_bytes: usize,
) -> io::Result<()> {
    let bytes = encode_frame(value, max_bytes)?;
    writer.write_all(&bytes).await?;
    writer.flush().await
}

pub fn encode_frame(value: &Value, max_bytes: usize) -> io::Result<Vec<u8>> {
    let mut writer = LimitedWriter {
        bytes: Vec::new(),
        max_bytes,
    };
    serde_json::to_writer(&mut writer, value).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidData, "Plugin frame exceeds the limit")
    })?;
    writer.bytes.push(b'\n');
    Ok(writer.bytes)
}

struct LimitedWriter {
    bytes: Vec<u8>,
    max_bytes: usize,
}

impl io::Write for LimitedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.bytes.len().saturating_add(bytes.len()) >= self.max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Plugin frame exceeds the limit",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
