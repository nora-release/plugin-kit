use crate::types::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, RwLock,
};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

const MAX_ACTIVE_REQUESTS: usize = 128;

#[cfg(test)]
#[path = "../test/client_observation.test.rs"]
mod client_observation_tests;

#[derive(Clone)]
struct SubscriptionPolicy {
    manifest: Value,
    enabled: bool,
}
impl Default for SubscriptionPolicy {
    fn default() -> Self {
        Self {
            manifest: Value::Null,
            enabled: true,
        }
    }
}

struct ActiveRequests(BTreeMap<u64, JoinHandle<std::io::Result<()>>>);
impl std::ops::Deref for ActiveRequests {
    type Target = BTreeMap<u64, JoinHandle<std::io::Result<()>>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for ActiveRequests {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl Drop for ActiveRequests {
    fn drop(&mut self) {
        for task in self.0.values() {
            task.abort();
        }
    }
}

struct CancelTaskOnDrop(tokio::task::AbortHandle);
impl Drop for CancelTaskOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Deserialize)]
struct Request {
    #[serde(default)]
    id: Option<u64>,
    method: String,
    #[serde(default)]
    params: Value,
}

pub async fn serve<P: ProviderPlugin>(plugin: P) -> std::io::Result<()> {
    serve_io(Arc::new(plugin), tokio::io::stdin(), tokio::io::stdout()).await
}

/// Serve the native plugin protocol. Each fetch is cancellable by an abort
/// request while its chunks are still being written to the same stdout pipe.
pub async fn serve_io<P, R, W>(plugin: Arc<P>, input: R, output: W) -> std::io::Result<()>
where
    P: ProviderPlugin + ?Sized,
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let mut reader = BufReader::new(input);
    let writer = Arc::new(Mutex::new(output));
    let initialized = Arc::new(AtomicBool::new(false));
    let mut requests = ActiveRequests(BTreeMap::new());
    let acknowledgments = Arc::new(crate::credential_updates::Acknowledgments::default());
    let credential_updates = Arc::new(AtomicBool::new(false));
    let http_acknowledgments = Arc::new(crate::request_meter::Acknowledgments::default());
    let http_meter = Arc::new(AtomicBool::new(false));
    let subscription_policy = Arc::new(RwLock::new(SubscriptionPolicy::default()));
    while let Some(bytes) = read_frame(&mut reader).await? {
        requests.retain(|_, task| !task.is_finished());
        let request = match serde_json::from_slice::<Request>(&bytes) {
            Ok(request) => request,
            Err(_) => {
                write_error(
                    &writer,
                    None,
                    PluginError::invalid("Invalid JSONL request."),
                )
                .await?;
                continue;
            }
        };
        if request.method == "abort" {
            let target = request.params.get("id").and_then(Value::as_u64);
            let aborted = target.and_then(|id| requests.remove(&id).map(|task| (id, task)));
            if let Some((id, task)) = aborted {
                task.abort();
                let _ = task.await;
                write_error(
                    &writer,
                    Some(id),
                    PluginError::new("cancelled", "The provider request was cancelled."),
                )
                .await?;
                if let Some(request_id) = request.id {
                    write_frame(&writer, json!({"id":request_id,"result":{"aborted":true}}))
                        .await?;
                }
            } else if let Some(request_id) = request.id {
                write_frame(&writer, json!({"id":request_id,"result":{"aborted":false}})).await?;
            }
            continue;
        }
        if request.method == "credentialsSaved" {
            acknowledgments.receive(&request.params);
            continue;
        }
        if request.method == "httpRequestAllowed" {
            http_acknowledgments.receive(&request.params);
            continue;
        }
        let Some(id) = request.id else {
            write_error(
                &writer,
                None,
                PluginError::invalid("A numeric request id is required."),
            )
            .await?;
            continue;
        };
        if requests.contains_key(&id) {
            write_error(
                &writer,
                Some(id),
                PluginError::invalid("The request id is already active."),
            )
            .await?;
            continue;
        }
        if requests.len() >= MAX_ACTIVE_REQUESTS {
            write_error(
                &writer,
                Some(id),
                PluginError::new("busy", "The provider plugin has too many active requests."),
            )
            .await?;
            continue;
        }
        let plugin = plugin.clone();
        let writer = writer.clone();
        let initialized = initialized.clone();
        let acknowledgments = acknowledgments.clone();
        let credential_updates = credential_updates.clone();
        let http_acknowledgments = http_acknowledgments.clone();
        let http_meter = http_meter.clone();
        let subscription_policy = subscription_policy.clone();
        requests.insert(
            id,
            tokio::spawn(async move {
                crate::credential_updates::scope(
                    id,
                    writer.clone(),
                    acknowledgments,
                    credential_updates.clone(),
                    crate::request_meter::scope(
                        id,
                        writer.clone(),
                        http_acknowledgments,
                        if matches!(
                            request.method.as_str(),
                            "fetch" | "webSearch" | "alphaSearch" | "resolveModel"
                        ) {
                            http_meter.clone()
                        } else {
                            Arc::new(AtomicBool::new(false))
                        },
                        run_request(
                            plugin,
                            &writer,
                            &initialized,
                            &credential_updates,
                            &http_meter,
                            &subscription_policy,
                            id,
                            &request.method,
                            request.params,
                        ),
                    ),
                )
                .await
            }),
        );
    }
    while let Some((_, task)) = requests.pop_first() {
        let _cancel_if_runtime_drops = CancelTaskOnDrop(task.abort_handle());
        match task.await {
            Ok(result) => result?,
            Err(error) if error.is_cancelled() => {}
            Err(_) => {
                return Err(std::io::Error::other(
                    "The provider plugin request task failed.",
                ))
            }
        }
    }
    Ok(())
}

async fn run_request<P, W>(
    plugin: Arc<P>,
    writer: &Arc<Mutex<W>>,
    initialized: &AtomicBool,
    credential_updates: &AtomicBool,
    http_meter: &AtomicBool,
    subscription_policy: &RwLock<SubscriptionPolicy>,
    id: u64,
    method: &str,
    params: Value,
) -> std::io::Result<()>
where
    P: ProviderPlugin + ?Sized,
    W: AsyncWrite + Unpin + Send + 'static,
{
    if method != "initialize" && !initialized.load(Ordering::Acquire) {
        return write_error(
            writer,
            Some(id),
            PluginError::invalid("Initialize the provider plugin before calling its methods."),
        )
        .await;
    }
    let mut policy = subscription_policy
        .read()
        .map_err(|_| std::io::Error::other("The provider policy is unavailable."))?
        .clone();
    if method == "initialize" {
        policy.manifest = params
            .get("installedManifest")
            .filter(|value| value.is_object())
            .or_else(|| params.get("manifest"))
            .and_then(|manifest| manifest.pointer("/contributes/provider"))
            .cloned()
            .unwrap_or(Value::Null);
    }
    if let Some(value) = params.get("subscriptionAccountsEnabled") {
        let Some(enabled) = value.as_bool() else {
            return write_error(
                writer,
                Some(id),
                PluginError::invalid("The subscription account setting is invalid."),
            )
            .await;
        };
        policy.enabled = enabled;
    }
    let enabled = policy.enabled && crate::subscription_policy::subscription_accounts_enabled();
    if !enabled
        && crate::subscription_policy::request_uses_subscription(&policy.manifest, method, &params)
    {
        return write_error(writer, Some(id), crate::subscription_policy::disabled()).await;
    }
    let automatic_proxy = params
        .get("networkProxyAutomatic")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let bypass_lan = params
        .get("networkProxyBypassLAN")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if method == "fetch" {
        let request = match serde_json::from_value::<FetchRequest>(params) {
            Ok(request) => request,
            Err(_) => {
                return write_error(
                    writer,
                    Some(id),
                    PluginError::invalid("Invalid HTTP fetch parameters."),
                )
                .await
            }
        };
        return crate::network::scope(
            automatic_proxy,
            bypass_lan,
            crate::subscription_policy::scope(enabled, async {
                match plugin.fetch(request).await {
                    Ok(response) => write_response(writer, id, response).await,
                    Err(error) => write_error(writer, Some(id), error).await,
                }
            }),
        )
        .await;
    }
    let accepts_credentials = params
        .pointer("/hostCapabilities/credentialUpdates")
        .and_then(Value::as_bool)
        == Some(true);
    let accepts_http_meter = params
        .pointer("/hostCapabilities/httpRequestMeter")
        .and_then(Value::as_bool)
        == Some(true);
    let mut result = crate::network::scope(
        automatic_proxy,
        bypass_lan,
        crate::subscription_policy::scope(enabled, dispatch(plugin.as_ref(), method, params)),
    )
    .await;
    if method == "initialize" && result.is_ok() {
        *subscription_policy
            .write()
            .map_err(|_| std::io::Error::other("The provider policy is unavailable."))? = policy;
        initialized.store(true, Ordering::Release);
        credential_updates.store(accepts_credentials, Ordering::Release);
        http_meter.store(accepts_http_meter, Ordering::Release);
        if let Ok(Value::Object(ref mut value)) = result {
            let capabilities = value.entry("capabilities").or_insert_with(|| json!({}));
            if !capabilities.is_object() {
                *capabilities = json!({});
            }
            if let Some(hooks) = plugin.gateway_hooks() {
                capabilities["gatewayHooks"] = hooks.capabilities();
            }
            capabilities["persistent"] = json!(true);
            capabilities["bodyFile"] = json!(true);
            capabilities["httpRequestMeter"] = json!(true);
            let features = capabilities
                .as_object_mut()
                .unwrap()
                .entry("sdkFeatures")
                .or_insert_with(|| json!({}));
            if !features.is_object() {
                *features = json!({});
            }
            features["subscriptionPolicy"] = json!(1);
            features["automaticProxy"] = json!(1);
            if plugin.supports_client_observation() {
                features["clientObservation"] = json!(1);
            } else if let Some(features) = features.as_object_mut() {
                features.remove("clientObservation");
            }
            if plugin.supports_account_revocation() {
                features["accountRevocation"] = json!(1);
            } else if let Some(features) = features.as_object_mut() {
                features.remove("accountRevocation");
            }
            if plugin.supports_learned_usage() {
                features["learningUsage"] = json!(1);
            } else if let Some(features) = features.as_object_mut() {
                features.remove("learningUsage");
            }
            if plugin.supports_session_activity() {
                features["sessionActivity"] = json!(1);
            } else if let Some(features) = features.as_object_mut() {
                features.remove("sessionActivity");
            }
            if plugin.supports_alpha_search() {
                features["alphaSearch"] = json!(1);
            } else if let Some(features) = features.as_object_mut() {
                features.remove("alphaSearch");
            }
            if plugin.supports_web_search() {
                features["webSearch"] = json!(1);
            } else if let Some(features) = features.as_object_mut() {
                features.remove("webSearch");
            }
            if plugin.model_resolution() {
                features["modelResolution"] = json!(1);
            } else if let Some(features) = features.as_object_mut() {
                features.remove("modelResolution");
            }
            capabilities
                .as_object_mut()
                .unwrap()
                .entry("accountDiscovery")
                .or_insert(json!(false));
        }
    }
    match result {
        Ok(result) if method == "gatewayHook" => {
            crate::gateway_hooks::transport::write_result(writer, id, result).await
        }
        Ok(result) => write_frame(writer, json!({"id":id,"result":result})).await,
        Err(error) => write_error(writer, Some(id), error).await,
    }
}

async fn dispatch<P: ProviderPlugin + ?Sized>(
    plugin: &P,
    method: &str,
    params: Value,
) -> PluginResult<Value> {
    match method {
        "initialize" => {
            let params: InitializeParams = serde_json::from_value(params)
                .map_err(|_| PluginError::invalid("Invalid plugin initialization parameters."))?;
            if params.api_version != API_VERSION {
                return Err(PluginError::invalid(
                    "Unsupported Nora provider plugin API version.",
                ));
            }
            plugin.initialize(params).await
        }
        "gatewayHook" => match plugin.gateway_hooks() {
            Some(hooks) => hooks.call(params).await,
            None => Err(PluginError::unsupported("gatewayHook")),
        },
        "resolveModel" => {
            if !plugin.model_resolution() {
                return Err(PluginError::unsupported("resolveModel"));
            }
            let request = serde_json::from_value(params)
                .map_err(|_| PluginError::invalid("Invalid model selection parameters."))?;
            serde_json::to_value(plugin.resolve_model(request).await?)
                .map_err(|_| PluginError::invalid("Invalid model selection result."))
        }
        "revokeAccount" => {
            if !plugin.supports_account_revocation() {
                return Err(PluginError::unsupported("revokeAccount"));
            }
            plugin.revoke_account(context(params)?).await?;
            Ok(json!({"ok":true}))
        }
        "observeClient" => {
            if !plugin.supports_client_observation() {
                return Err(PluginError::unsupported("observeClient"));
            }
            let mut params = params;
            if let Some(policy) = params.get("subscriptionAccountsEnabled") {
                if !policy.is_boolean() {
                    return Err(PluginError::invalid(
                        "The subscription account setting is invalid.",
                    ));
                }
                params
                    .as_object_mut()
                    .unwrap()
                    .remove("subscriptionAccountsEnabled");
            }
            let observation: crate::ClientObservation = serde_json::from_value(params)
                .map_err(|_| PluginError::invalid("The caller identity is invalid."))?;
            observation.validate()?;
            plugin.observe_client(observation).await?;
            Ok(json!({"ok":true}))
        }
        "learnedUsage" => {
            if !plugin.supports_learned_usage() {
                return Err(PluginError::unsupported("learnedUsage"));
            }
            serde_json::to_value(plugin.learned_usage(context(params)?).await?)
                .map_err(|_| PluginError::invalid("The observed allowance is invalid."))
        }
        "sessionActivity" => {
            if !plugin.supports_session_activity() {
                return Err(PluginError::unsupported("sessionActivity"));
            }
            serde_json::to_value(plugin.session_activity(context(params)?).await?)
                .map_err(|_| PluginError::invalid("The session activity is invalid."))
        }
        "alphaSearch" => {
            if !plugin.supports_alpha_search() {
                return Err(PluginError::unsupported("alphaSearch"));
            }
            let request = params
                .get("request")
                .filter(|value| value.is_object())
                .ok_or_else(|| PluginError::invalid("Provide a search request object."))?
                .clone();
            let headers =
                serde_json::from_value(params.get("headers").cloned().unwrap_or(json!({})))
                    .map_err(|_| PluginError::invalid("Provide valid search request headers."))?;
            plugin
                .alpha_search(context(params)?, request, headers)
                .await
        }
        "webSearch" => {
            if !plugin.supports_web_search() {
                return Err(PluginError::unsupported("webSearch"));
            }
            let query = params
                .get("query")
                .and_then(Value::as_str)
                .filter(|query| {
                    !query.trim().is_empty() && query.len() <= 16384 && !query.contains('\0')
                })
                .ok_or_else(|| {
                    PluginError::invalid("Provide a nonempty search query of at most 16384 bytes.")
                })?
                .to_owned();
            plugin.web_search(context(params)?, query).await
        }
        "models" => plugin.models(context(params)?).await,
        "authMethods" => plugin.auth_methods(context(params)?).await,
        "accounts" => plugin.accounts(context(params)?).await,
        "discoverAccounts" => plugin.discover_accounts(context(params)?).await,
        "authorize" => plugin.authorize(params).await,
        "callback" => plugin.callback(params).await,
        "refresh" => plugin.refresh(params).await,
        "usage" => plugin.usage(context(params)?).await,
        other => plugin.call(other, params).await,
    }
}

fn context(params: Value) -> PluginResult<CallContext> {
    if params.is_null() {
        return Ok(CallContext::default());
    }
    serde_json::from_value(params)
        .map_err(|_| PluginError::invalid("Invalid provider account parameters."))
}

async fn write_response<W: AsyncWrite + Unpin + Send + 'static>(
    writer: &Arc<Mutex<W>>,
    id: u64,
    mut response: FetchResponse,
) -> std::io::Result<()> {
    if !(100..=599).contains(&response.status) {
        return write_error(
            writer,
            Some(id),
            PluginError::invalid("The provider returned an invalid HTTP status."),
        )
        .await;
    }
    write_frame(
        writer,
        json!({"id":id,"event":"head","status":response.status,"headers":response.headers}),
    )
    .await?;
    while let Some(chunk) = response.body.next().await {
        let bytes = match chunk {
            Ok(bytes) => bytes,
            Err(error) => return write_error(writer, Some(id), error).await,
        };
        for bytes in bytes.chunks(MAX_HTTP_CHUNK_BYTES) {
            write_frame(
                writer,
                json!({"id":id,"event":"chunk","data":STANDARD.encode(bytes)}),
            )
            .await?;
        }
    }
    write_frame(writer, json!({"id":id,"event":"done"})).await
}

async fn write_error<W: AsyncWrite + Unpin + Send + 'static>(
    writer: &Arc<Mutex<W>>,
    id: Option<u64>,
    error: PluginError,
) -> std::io::Result<()> {
    write_frame(writer, json!({"id":id,"error":error})).await
}

pub(crate) async fn write_frame<W: AsyncWrite + Unpin + Send + 'static>(
    writer: &Arc<Mutex<W>>,
    mut value: Value,
) -> std::io::Result<()> {
    let mut bytes = serde_json::to_vec(&value).map_err(std::io::Error::other)?;
    if bytes.len() > MAX_RESPONSE_FRAME_BYTES {
        let id = value.get("id").cloned().unwrap_or(Value::Null);
        value = json!({"id":id,"error":{"code":"response_too_large","message":"The provider plugin response frame exceeds 1 MiB."}});
        bytes = serde_json::to_vec(&value).map_err(std::io::Error::other)?;
    }
    bytes.push(b'\n');
    let mut writer = writer.lock().await;
    writer.write_all(&bytes).await?;
    writer.flush().await
}

async fn read_frame<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
) -> std::io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    loop {
        let buffer = reader.fill_buf().await?;
        if buffer.is_empty() {
            return if bytes.is_empty() {
                Ok(None)
            } else {
                Ok(Some(bytes))
            };
        }
        let count = buffer
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| index + 1)
            .unwrap_or(buffer.len());
        if bytes.len().saturating_add(count) > MAX_REQUEST_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "The provider plugin request exceeds 32 MiB.",
            ));
        }
        let complete = buffer[count - 1] == b'\n';
        bytes.extend_from_slice(&buffer[..count]);
        reader.consume(count);
        if complete {
            return Ok(Some(bytes));
        }
    }
}

#[cfg(test)]
#[path = "../test/runtime.test.rs"]
mod tests;

#[cfg(test)]
#[path = "../test/account_revocation.test.rs"]
mod account_revocation_tests;
