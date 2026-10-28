//! Rust provider plugins run as child processes managed by Nora. stdout is
//! reserved for JSONL protocol frames; diagnostics belong on stderr.

mod client_observation;
pub mod connection;
mod credential_updates;
pub mod gateway_hooks;
pub mod gateway_middleware;
pub mod model_resolution;
pub mod network;
mod request_body;
mod request_meter;
mod runtime;
pub mod subscription_policy;
mod types;

pub use client_observation::ClientObservation;
pub use credential_updates::save_credentials;
pub use network::send_http;
pub use network::{http_client, http_client_builder, NetworkProxy, NetworkProxyView};
pub use request_body::{RequestBodyFile, MAX_BODY_FILE_BYTES};
pub use request_meter::{before_http_request, RequestMeterContext};
pub use runtime::{serve, serve_io};
pub use subscription_policy::{require_subscription_accounts, subscription_accounts_enabled};
pub use types::{
    CallContext, FetchRequest, FetchResponse, InitializeParams, PluginError, PluginResult,
    ProviderPlugin, RequestConstraints, ResponseBody, SessionActivity, API_VERSION,
    MAX_HTTP_CHUNK_BYTES, MAX_REQUEST_BYTES, MAX_RESPONSE_FRAME_BYTES,
};
