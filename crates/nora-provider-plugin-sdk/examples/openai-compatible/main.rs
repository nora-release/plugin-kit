use async_trait::async_trait;
use futures::StreamExt;
use nora_provider_plugin_sdk::{
    serve, CallContext, FetchRequest, FetchResponse, InitializeParams, PluginError, PluginResult,
    ProviderPlugin, API_VERSION,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::RwLock;

#[derive(Clone)]
struct Config {
    base_url: String,
    models: Vec<Value>,
}

struct OpenAICompatible {
    config: RwLock<Option<Config>>,
}

impl OpenAICompatible {
    fn config(&self) -> PluginResult<Config> {
        self.config
            .read()
            .map_err(|_| {
                PluginError::new(
                    "configuration_error",
                    "The provider configuration is unavailable.",
                )
            })?
            .clone()
            .ok_or_else(|| {
                PluginError::new("not_initialized", "The provider plugin is not initialized.")
            })
    }
}

#[async_trait]
impl ProviderPlugin for OpenAICompatible {
    async fn initialize(&self, params: InitializeParams) -> PluginResult<Value> {
        let provider = params.provider()?;
        let config = Config {
            base_url: provider.base_url,
            models: provider.models,
        };
        let url = reqwest::Url::parse(&config.base_url)
            .map_err(|_| PluginError::invalid("baseURL is invalid."))?;
        if !matches!(url.scheme(), "https" | "http")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(PluginError::invalid(
                "baseURL must be an HTTP address without credentials, query or fragment.",
            ));
        }
        if config.models.is_empty() {
            return Err(PluginError::invalid(
                "Configure at least one model in nora-plugin.json.",
            ));
        }
        *self.config.write().map_err(|_| {
            PluginError::new(
                "configuration_error",
                "Cannot update the provider configuration.",
            )
        })? = Some(config);
        Ok(json!({"apiVersion":API_VERSION}))
    }

    async fn models(&self, _context: CallContext) -> PluginResult<Value> {
        Ok(json!(self.config()?.models))
    }

    async fn auth_methods(&self, _context: CallContext) -> PluginResult<Value> {
        Ok(
            json!([{"id":"apiKey","type":"api","label":"API key","fields":[{"id":"apiKey","type":"password","label":"API key","required":true}]}]),
        )
    }

    async fn authorize(&self, params: Value) -> PluginResult<Value> {
        let key = params
            .pointer("/inputs/apiKey")
            .or_else(|| params.pointer("/fields/apiKey"))
            .or_else(|| params.get("apiKey"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|key| !key.is_empty() && !key.contains(['\r', '\n']))
            .ok_or_else(|| PluginError::authentication("Enter the provider's API key."))?;
        Ok(json!({"ok":true,"credentials":{"type":"api","key":key}}))
    }

    async fn fetch(&self, request: FetchRequest) -> PluginResult<FetchResponse> {
        let config = self.config()?;
        let base = reqwest::Url::parse(&config.base_url)
            .map_err(|_| PluginError::invalid("baseURL is invalid."))?;
        let target = reqwest::Url::parse(&request.url)
            .map_err(|_| PluginError::invalid("The request URL is invalid."))?;
        let base_path = base.path().trim_end_matches('/');
        if base.origin() != target.origin()
            || !(target.path() == base_path || target.path().starts_with(&format!("{base_path}/")))
            || !target.username().is_empty()
            || target.password().is_some()
        {
            return Err(PluginError::invalid(
                "The request address is outside this provider's configured baseURL.",
            ));
        }
        let key = request
            .credentials
            .as_ref()
            .and_then(|credentials| credentials.get("key").or_else(|| credentials.get("access")))
            .and_then(Value::as_str)
            .filter(|key| !key.trim().is_empty() && !key.contains(['\r', '\n']))
            .ok_or_else(|| {
                PluginError::authentication("Add an API key for this provider in Nora.")
            })?
            .to_string();
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|_| PluginError::invalid("The HTTP method is invalid."))?;
        let body = request.decode_body()?;
        let client = nora_provider_plugin_sdk::http_client(request.network_proxy.as_ref())?;
        let mut outgoing = client.request(method, target);
        for (name, value) in request.headers {
            if matches!(
                name.to_ascii_lowercase().as_str(),
                "authorization"
                    | "x-api-key"
                    | "api-key"
                    | "host"
                    | "content-length"
                    | "connection"
                    | "transfer-encoding"
            ) {
                continue;
            }
            outgoing = outgoing.header(name, value);
        }
        outgoing = outgoing.bearer_auth(&key);
        if let Some(body) = body {
            outgoing = outgoing.body(body);
        }
        let response = nora_provider_plugin_sdk::send_http(outgoing)
            .await
            .map_err(|error| {
                PluginError::transport(error.to_string().replace(&key, "[redacted]"))
            })?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.to_string(), value.to_string()))
            })
            .collect::<BTreeMap<_, _>>();
        let body = response.bytes_stream().map(move |chunk| {
            chunk.map(|bytes| bytes.to_vec()).map_err(|error| {
                PluginError::transport(error.to_string().replace(&key, "[redacted]"))
            })
        });
        Ok(FetchResponse {
            status,
            headers,
            body: Box::pin(body),
        })
    }
}

#[tokio::main]
async fn main() {
    let plugin = OpenAICompatible {
        config: RwLock::new(None),
    };
    if let Err(error) = serve(plugin).await {
        eprintln!("Provider protocol stopped: {error}");
        std::process::exit(1);
    }
}
