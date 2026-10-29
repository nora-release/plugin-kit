use super::*;
use crate::{RequestBodyFile, MAX_HTTP_CHUNK_BYTES};
use base64::{engine::general_purpose::STANDARD, Engine};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWrite;
use tokio::sync::Mutex as AsyncMutex;

pub(crate) const INLINE_BODY_BYTES: usize = 512 * 1024;

pub(super) async fn hydrate(mut params: Value) -> PluginResult<Value> {
    let fields: &[(&str, &str)] = match params.get("operation").and_then(Value::as_str) {
        Some("begin") => &[("body", "bodyFile")],
        Some("finish") => &[("request", "requestFile"), ("reply", "replyFile")],
        _ => return Ok(params),
    };
    for (field, file_field) in fields {
        let Some(descriptor) = params.get(*file_field) else {
            continue;
        };
        if params.get(*field).is_some() {
            return Err(invalid());
        }
        let descriptor: RequestBodyFile =
            serde_json::from_value(descriptor.clone()).map_err(|_| invalid())?;
        if descriptor.length > MAX_BODY_BYTES as u64 {
            return Err(invalid());
        }
        let value = tokio::task::spawn_blocking(move || {
            let bytes = descriptor.read_verified()?;
            let value = serde_json::from_slice::<Value>(&bytes).map_err(|_| invalid())?;
            if !value.is_object() {
                return Err(invalid());
            }
            Ok(value)
        })
        .await
        .map_err(|_| unavailable())??;
        params
            .as_object_mut()
            .ok_or_else(invalid)?
            .remove(*file_field);
        params[*field] = value;
    }
    Ok(params)
}

/// Only this private RPC streams large JSON bodies. Standard result frames
/// keep their one-MiB limit; each streamed frame is at most one HTTP chunk.
pub(crate) async fn write_result<W>(
    writer: &Arc<AsyncMutex<W>>,
    id: u64,
    mut result: Value,
) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    let body = result
        .get("body")
        .filter(|body| !fits(body, INLINE_BODY_BYTES));
    let Some(body) = body else {
        return crate::runtime::write_frame(writer, json!({"id":id,"result":result})).await;
    };
    let bytes = serde_json::to_vec(body).map_err(std::io::Error::other)?;
    if bytes.len() > MAX_BODY_BYTES {
        return crate::runtime::write_frame(writer, json!({"id":id,"error":invalid()})).await;
    }
    let checksum = format!("{:x}", Sha256::digest(&bytes));
    result
        .as_object_mut()
        .ok_or_else(|| std::io::Error::other("Invalid gateway result"))?
        .remove("body");
    for (sequence, chunk) in bytes.chunks(MAX_HTTP_CHUNK_BYTES).enumerate() {
        crate::runtime::write_frame(writer,json!({"id":id,"event":"gatewayHookBody","sequence":sequence,"data":STANDARD.encode(chunk)})).await?;
    }
    result["bodyStream"] = json!({"length":bytes.len(),"sha256":checksum});
    crate::runtime::write_frame(writer, json!({"id":id,"result":result})).await
}
