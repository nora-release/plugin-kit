use futures::Stream;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::pin::Pin;

pub const API_VERSION: u32 = 1;
pub const MAX_REQUEST_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_RESPONSE_FRAME_BYTES: usize = 1024 * 1024;
pub const MAX_HTTP_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionActivity {
    pub tools: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    pub api_version: u32,
    #[serde(
        default = "default_subscription_accounts",
        rename = "subscriptionAccountsEnabled"
    )]
    pub subscription_accounts_enabled: bool,
    #[serde(default, rename = "pluginID")]
    pub plugin_id: String,
    pub plugin_directory: PathBuf,
    pub data_directory: PathBuf,
    pub manifest: Value,
    /// The installed package before the host applies user connection overrides.
    #[serde(default)]
    pub installed_manifest: Option<Value>,
    /// Explicit host settings, applied after selecting an account's API profile.
    #[serde(default, rename = "config")]
    pub runtime_config: Value,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CallContext {
    /// Read-only public model metadata from catalogs already loaded by the host.
    /// This never includes provider credentials or starts another account session.
    #[serde(default, rename = "publicModelCatalogs")]
    pub public_model_catalogs: std::collections::BTreeMap<String, Vec<Value>>,
    #[serde(
        default = "default_subscription_accounts",
        rename = "subscriptionAccountsEnabled"
    )]
    pub subscription_accounts_enabled: bool,
    #[serde(default = "default_account")]
    pub account: String,
    #[serde(default)]
    pub credentials: Option<Value>,
    #[serde(default, rename = "networkProxy")]
    pub network_proxy: Option<crate::NetworkProxy>,
}

impl Default for CallContext {
    fn default() -> Self {
        Self {
            public_model_catalogs: Default::default(),
            subscription_accounts_enabled: true,
            account: default_account(),
            credentials: None,
            network_proxy: None,
        }
    }
}
fn default_account() -> String {
    "default".to_string()
}
fn default_subscription_accounts() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize)]
pub struct FetchRequest {
    /// User-Agent from the public caller, kept separate from codec headers.
    /// Plugins decide whether their upstream accepts the caller's identity.
    #[serde(default, rename = "callerUserAgent")]
    pub caller_user_agent: Option<String>,
    /// Host-selected request policy. This is private to the host/plugin protocol
    /// and must never be inferred from public HTTP bodies or request metadata.
    #[serde(default)]
    pub constraints: RequestConstraints,
    #[serde(
        default = "default_subscription_accounts",
        rename = "subscriptionAccountsEnabled"
    )]
    pub subscription_accounts_enabled: bool,
    #[serde(default = "default_account")]
    pub account: String,
    #[serde(default)]
    pub credentials: Option<Value>,
    #[serde(default, rename = "networkProxy")]
    pub network_proxy: Option<crate::NetworkProxy>,
    pub url: String,
    pub method: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// Host routing identifiers for suppliers with persistent chat sessions.
    /// These values are separate from HTTP headers and must not be sent upstream.
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    /// Base64 request bytes. Use decode_body() before sending the request.
    pub body: Option<String>,
    /// A host-owned request file; it remains available until this fetch ends.
    /// This field is private to the host/plugin protocol, never a public HTTP input.
    /// The SDK announces support to Nora; fetch implementations must use decode_body()
    /// or this descriptor's verified streaming helpers instead of reading body alone.
    #[serde(default, rename = "bodyFile")]
    pub body_file: Option<crate::RequestBodyFile>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestConstraints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_token_limit: Option<u64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reasoning_disabled: bool,
}

impl FetchRequest {
    pub fn decode_body(&self) -> PluginResult<Option<Vec<u8>>> {
        use base64::Engine;
        if let Some(file) = &self.body_file {
            if self.body.is_some() {
                return Err(PluginError::invalid(
                    "Provide one provider request body source.",
                ));
            }
            return file.read_verified().map(Some);
        }
        self.body
            .as_deref()
            .map(|body| {
                base64::engine::general_purpose::STANDARD
                    .decode(body)
                    .map_err(|_| PluginError::invalid("The HTTP request body is not valid base64."))
            })
            .transpose()
    }
}

pub type ResponseBody = Pin<Box<dyn Stream<Item = PluginResult<Vec<u8>>> + Send + 'static>>;

pub struct FetchResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: ResponseBody,
}

impl FetchResponse {
    pub fn bytes(status: u16, headers: BTreeMap<String, String>, body: Vec<u8>) -> Self {
        Self {
            status,
            headers,
            body: Box::pin(futures::stream::once(async move { Ok(body) })),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct PluginError {
    pub code: String,
    pub message: String,
}

impl PluginError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid_request", message)
    }
    pub fn unsupported(method: &str) -> Self {
        Self::new(
            "method_not_supported",
            format!("This provider plugin does not support {method}."),
        )
    }
    pub fn authentication(message: impl Into<String>) -> Self {
        Self::new("authentication_required", message)
    }
    pub fn transport(message: impl Into<String>) -> Self {
        Self::new("upstream_transport_error", message)
    }
}
impl std::fmt::Display for PluginError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}
impl std::error::Error for PluginError {}
pub type PluginResult<T> = Result<T, PluginError>;

#[async_trait::async_trait]
pub trait ProviderPlugin: Send + Sync + 'static {
    fn supports_client_observation(&self) -> bool {
        false
    }

    /// Observe public client identity without reading an account or using HTTP.
    async fn observe_client(&self, _observation: crate::ClientObservation) -> PluginResult<()> {
        Err(PluginError::unsupported("observeClient"))
    }

    fn supports_account_revocation(&self) -> bool {
        false
    }

    /// Revoke the supplied sign-in after its local account has been removed.
    /// This is best effort and must not renew or save credentials.
    async fn revoke_account(&self, _context: CallContext) -> PluginResult<()> {
        Err(PluginError::unsupported("revokeAccount"))
    }

    fn supports_learned_usage(&self) -> bool {
        false
    }

    fn supports_session_activity(&self) -> bool {
        false
    }

    /// Read resident sessions in this process; this must not start a session or contact a vendor.
    async fn session_activity(&self, _context: CallContext) -> PluginResult<SessionActivity> {
        Ok(SessionActivity::default())
    }

    /// Read allowance windows observed in completed or ongoing responses.
    /// Implementations must not make a vendor request to answer this query.
    /// A result includes `items` and the original RFC3339 `observedAt` time;
    /// individual windows may carry their own `observedAt` for partial updates.
    /// Reading the cache must never advance these observation times.
    async fn learned_usage(&self, _context: CallContext) -> PluginResult<Option<Value>> {
        Ok(None)
    }

    fn supports_alpha_search(&self) -> bool {
        false
    }

    async fn alpha_search(
        &self,
        _context: CallContext,
        _request: Value,
        _headers: BTreeMap<String, String>,
    ) -> PluginResult<Value> {
        Err(PluginError::unsupported("alphaSearch"))
    }

    fn supports_web_search(&self) -> bool {
        false
    }

    async fn web_search(&self, _context: CallContext, _query: String) -> PluginResult<Value> {
        Err(PluginError::unsupported("webSearch"))
    }

    fn model_resolution(&self) -> bool {
        false
    }

    async fn resolve_model(
        &self,
        _request: crate::model_resolution::ResolveModelRequest,
    ) -> PluginResult<Option<crate::model_resolution::ModelResolution>> {
        Ok(None)
    }

    fn gateway_hooks(&self) -> Option<&crate::gateway_hooks::GatewayHooks> {
        None
    }

    async fn initialize(&self, params: InitializeParams) -> PluginResult<Value> {
        if params.api_version != API_VERSION {
            return Err(PluginError::invalid(
                "Unsupported Nora provider plugin API version.",
            ));
        }
        Ok(json!({"apiVersion":API_VERSION}))
    }
    async fn models(&self, _context: CallContext) -> PluginResult<Value> {
        Ok(json!([]))
    }
    async fn auth_methods(&self, _context: CallContext) -> PluginResult<Value> {
        Ok(json!([]))
    }
    /// Nora's Rust host owns saved accounts; override only for extra display metadata.
    async fn accounts(&self, _context: CallContext) -> PluginResult<Value> {
        Ok(json!([]))
    }
    /// Read existing local sign-ins without opening a browser or creating a login.
    /// Credentials remain private to the host. Missing sign-ins return an empty
    /// list; temporary failures must be reported rather than treated as sign-out.
    async fn discover_accounts(&self, _context: CallContext) -> PluginResult<Value> {
        Ok(json!({"accounts":[]}))
    }
    async fn authorize(&self, _params: Value) -> PluginResult<Value> {
        Err(PluginError::unsupported("authorize"))
    }
    async fn callback(&self, _params: Value) -> PluginResult<Value> {
        Err(PluginError::unsupported("callback"))
    }
    /// Return {"credentials": {...}} to replace renewed values atomically in Nora.
    async fn refresh(&self, _params: Value) -> PluginResult<Value> {
        Ok(json!({"changed":false}))
    }
    async fn usage(&self, _context: CallContext) -> PluginResult<Value> {
        Err(PluginError::unsupported("usage"))
    }
    async fn fetch(&self, _request: FetchRequest) -> PluginResult<FetchResponse> {
        Err(PluginError::unsupported("fetch"))
    }
    async fn call(&self, method: &str, _params: Value) -> PluginResult<Value> {
        Err(PluginError::unsupported(method))
    }
}
