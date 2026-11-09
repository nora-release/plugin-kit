//! Versioned JSON-RPC contracts shared by plugin hosts and language SDKs.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The version a host speaks unless the plugin's manifest names another.
pub const PROTOCOL_VERSION: u32 = 1;
/// The version that adds streams (`streams/data`, `streams/credit`).
pub const STREAMS_PROTOCOL_VERSION: u32 = 2;
pub const JSONRPC_VERSION: &str = "2.0";
pub const DEFAULT_MAX_FRAME_BYTES: usize = 1024 * 1024;
pub const DEFAULT_MAX_IN_FLIGHT: usize = 32;
pub const DEFAULT_CALL_TIMEOUT_MS: u64 = 600_000;
pub const MAX_CALL_TIMEOUT_MS: u64 = 3_600_000;
pub const INITIALIZE: &str = "initialize";
pub const CALL_TOOL: &str = "tools/call";
pub const CALL_SERVICE: &str = "services/call";
pub const CANCELLED: &str = "notifications/cancelled";
pub const SHUTDOWN: &str = "shutdown";
pub const STREAM_DATA: &str = "streams/data";
pub const STREAM_CREDIT: &str = "streams/credit";
/// A reader's starting credit for each stream unless it names its own.
pub const DEFAULT_STREAM_WINDOW: u64 = 1024 * 1024;
pub const MIN_STREAM_WINDOW: u64 = 64 * 1024;
pub const MAX_STREAM_WINDOW: u64 = 64 * 1024 * 1024;
/// What each `streams/data` costs besides its bytes, so a flood of small
/// frames cannot fill the reader's queue.
pub const STREAM_FRAME_COST: u64 = 1024;
/// The most raw bytes a writer puts in one `streams/data`.
pub const MAX_STREAM_CHUNK: usize = 256 * 1024;

/// Whether this crate speaks `version`.
pub fn supported_protocol(version: u32) -> bool {
    (PROTOCOL_VERSION..=STREAMS_PROTOCOL_VERSION).contains(&version)
}

/// What one `streams/data` carrying `bytes` raw bytes costs the writer.
pub fn stream_frame_cost(bytes: usize) -> u64 {
    u64::try_from(bytes)
        .unwrap_or(u64::MAX)
        .saturating_add(STREAM_FRAME_COST)
}

pub fn valid_service_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl Request {
    pub fn new(id: u64, method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.into(),
            id: Some(id),
            method: method.into(),
            params,
        }
    }

    pub fn notification(method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.into(),
            id: None,
            method: method.into(),
            params,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub jsonrpc: String,
    pub id: Option<u64>,
    #[serde(
        default,
        deserialize_with = "present_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

fn present_value<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

impl Response {
    pub fn success(id: u64, result: Value) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.into(),
            id: Some(id),
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Option<u64>, error: RpcError) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.into(),
            id,
            result: None,
            error: Some(error),
        }
    }

    pub fn is_valid(&self) -> bool {
        self.jsonrpc == JSONRPC_VERSION && (self.result.is_some() != self.error.is_some())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(-32602, message)
    }

    pub fn method_not_found() -> Self {
        Self::new(-32601, "The plugin method is not supported")
    }

    pub fn permission_denied() -> Self {
        Self::new(-32003, "The plugin has no grant for this service")
    }
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{} ({})", self.message, self.code)
    }
}

impl std::error::Error for RpcError {}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InitializeParams {
    pub protocol_version: u32,
    pub plugin_id: String,
    #[serde(default)]
    pub config: Value,
    #[serde(default)]
    pub granted_services: Vec<String>,
    /// Version 2: the host's starting credit for each stream it reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_window: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InitializeResult {
    pub protocol_version: u32,
    #[serde(default)]
    pub tools: Vec<ToolDefinition>,
    #[serde(default)]
    pub services: Vec<String>,
    /// Version 2: the plugin's starting credit for each stream it reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_window: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallToolParams {
    pub name: String,
    pub arguments: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_stream: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_stream: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolResult {
    #[serde(default)]
    pub content: Vec<ContentBlock>,
    #[serde(default)]
    pub is_error: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl ToolResult {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![ContentBlock::Text { text: text.into() }],
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Image {
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceCallParams {
    pub service: String,
    pub method: String,
    #[serde(default)]
    pub arguments: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_request_id: Option<u64>,
    /// Version 2: the stream the caller writes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_stream: Option<u64>,
    /// Version 2: the stream the callee writes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_stream: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CancelledParams {
    pub request_id: u64,
}

/// `streams/data`: raw bytes, base64 encoded, from a stream's writer.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StreamDataParams {
    pub stream: u64,
    #[serde(default)]
    pub data: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub end: bool,
    /// With `end`: the writer gave up, and the reader takes this as the
    /// read's error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

/// `streams/credit`: more bytes the reader takes.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StreamCreditParams {
    pub stream: u64,
    pub bytes: u64,
}

#[cfg(test)]
#[path = "../test/protocol.test.rs"]
mod tests;
