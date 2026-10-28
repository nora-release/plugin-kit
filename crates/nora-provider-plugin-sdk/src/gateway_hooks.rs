//! Request hooks are private gateway extensions. They receive protocol bodies,
//! never provider credentials or HTTP authorization headers.

pub(crate) mod transport;
use crate::{PluginError, PluginResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_STATE_BYTES: usize = 64 * 1024;
pub const MAX_OPTIONS_BYTES: usize = 32 * 1024;
pub const MAX_RUNS: usize = 128;
pub const RUN_TTL: Duration = Duration::from_secs(15 * 60);

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Context {
    #[serde(default)]
    pub origin: String,
    pub protocol: String,
    pub model: String,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub agent: String,
    #[serde(default)]
    pub session: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub query: String,
    #[serde(skip)]
    pub report_state: Reporter,
}

#[derive(Clone, Copy, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Ready,
    Empty,
    Unbound,
    Unauthorized,
    Unavailable,
    Invalid,
    #[default]
    Skipped,
}
#[derive(Clone, Default, Serialize)]
struct Reported {
    outcome: Outcome,
    bytes: usize,
}
#[derive(Clone, Default)]
pub struct Reporter(Arc<Mutex<Reported>>);
impl Context {
    /// Reports accept fixed outcomes and bounded byte counts, never free text.
    pub fn report(&self, outcome: Outcome, bytes: usize) {
        if let Ok(mut report) = self.report_state.0.lock() {
            *report = Reported {
                outcome,
                bytes: if matches!(outcome, Outcome::Ready) && bytes <= 32768 {
                    bytes
                } else {
                    0
                },
            };
        }
    }
    pub fn validate(&self) -> PluginResult<()> {
        if !matches!(
            self.origin.as_str(),
            "local" | "remote" | "gateway-key" | "unknown"
        ) || self.protocol.is_empty()
            || self.protocol.len() > 64
            || self.model.len() > 1024
            || self.path.len() > 2048
            || self.agent.len() > 1024
            || self.session.len() > 1024
            || self.cwd.len() > 4096
            || self.query.len() > 16 * 1024
            || [
                &self.origin,
                &self.protocol,
                &self.model,
                &self.path,
                &self.agent,
                &self.session,
                &self.cwd,
            ]
            .iter()
            .any(|text| text.chars().any(char::is_control))
        {
            return Err(invalid());
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct Completion {
    pub context: Context,
    pub request: Value,
    pub reply: Option<Value>,
    pub success: bool,
}

#[async_trait]
pub trait GatewayHook: Send + Sync + 'static {
    /// Returning None preserves the request. State belongs to this one request
    /// and is available only to its completion callback.
    async fn on_request(
        &self,
        context: &Context,
        body: Value,
        state: &mut Value,
        options: &Value,
    ) -> PluginResult<Option<Value>>;

    async fn on_completion(
        &self,
        _completion: &Completion,
        _state: &mut Value,
        _options: &Value,
    ) -> PluginResult<()> {
        Ok(())
    }
}

struct Entry {
    token: String,
    context: Context,
    state: Value,
    options: Value,
    request: Value,
    created: Instant,
    timeout: Duration,
    _permit: OwnedSemaphorePermit,
}

pub struct GatewayHooks {
    hook: Arc<dyn GatewayHook>,
    completion: bool,
    runs: Arc<Mutex<BTreeMap<String, Entry>>>,
    capacity: Arc<Semaphore>,
}

impl GatewayHooks {
    pub fn new(hook: impl GatewayHook) -> Self {
        Self {
            hook: Arc::new(hook),
            completion: true,
            runs: Arc::default(),
            capacity: Arc::new(Semaphore::new(MAX_RUNS)),
        }
    }

    pub fn request_only(mut self) -> Self {
        self.completion = false;
        self
    }

    pub fn capabilities(&self) -> Value {
        json!({"request":true,"completion":self.completion,"bodyFile":true,"bodyStream":true})
    }

    pub async fn call(&self, params: Value) -> PluginResult<Value> {
        self.prune()?;
        match params.get("operation").and_then(Value::as_str) {
            Some("begin") => {
                let permit = self
                    .capacity
                    .clone()
                    .try_acquire_owned()
                    .map_err(|_| PluginError::new("busy", "Gateway request hooks are busy."))?;
                self.begin(transport::hydrate(params).await?, permit).await
            }
            Some("finish") => {
                let Some(run) = self.take(&params)? else {
                    return Ok(Value::Null);
                };
                self.finish(transport::hydrate(params).await?, run).await
            }
            Some("close") => {
                self.take(&params)?;
                Ok(Value::Null)
            }
            _ => Err(invalid()),
        }
    }

    fn prune(&self) -> PluginResult<()> {
        self.runs
            .lock()
            .map_err(|_| unavailable())?
            .retain(|_, run| run.created.elapsed() < RUN_TTL);
        Ok(())
    }

    async fn begin(&self, params: Value, permit: OwnedSemaphorePermit) -> PluginResult<Value> {
        let context: Context =
            serde_json::from_value(params.get("context").cloned().ok_or_else(invalid)?)
                .map_err(|_| invalid())?;
        context.validate()?;
        let body = params
            .get("body")
            .filter(|body| body.is_object())
            .cloned()
            .ok_or_else(invalid)?;
        if !fits(&body, MAX_BODY_BYTES) {
            return Err(invalid());
        }
        let options = params.get("options").cloned().unwrap_or_else(|| json!({}));
        if !options.is_object() || !fits(&options, MAX_OPTIONS_BYTES) {
            return Err(invalid());
        }
        let timeout = timeout(&params)?;
        let mut state = json!({});
        let result = tokio::time::timeout(
            timeout,
            self.hook
                .on_request(&context, body.clone(), &mut state, &options),
        )
        .await
        .map_err(|_| PluginError::new("timeout", "The gateway request hook timed out."))?
        .map_err(|_| failed())?;
        let changed = result.is_some();
        let next = result.unwrap_or_else(|| body.clone());
        validate_change(&body, &next)?;
        let mut report = context
            .report_state
            .0
            .lock()
            .map_err(|_| unavailable())?
            .clone();
        if changed && matches!(report.outcome, Outcome::Skipped) {
            report.outcome = Outcome::Ready;
        }
        if !fits(&state, MAX_STATE_BYTES) {
            return Err(invalid());
        }
        let id = random_id()?;
        let token = random_id()?;
        self.runs.lock().map_err(|_| unavailable())?.insert(
            id.clone(),
            Entry {
                token: token.clone(),
                context,
                state,
                options,
                request: next.clone(),
                created: Instant::now(),
                timeout,
                _permit: permit,
            },
        );
        let weak = Arc::downgrade(&self.runs);
        let expired_id = id.clone();
        let expired_token = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(RUN_TTL).await;
            if let Some(runs) = weak.upgrade() {
                if let Ok(mut runs) = runs.lock() {
                    if runs
                        .get(&expired_id)
                        .is_some_and(|run| constant_time_equal(&run.token, &expired_token))
                    {
                        runs.remove(&expired_id);
                    }
                }
            }
        });
        Ok(json!({"runID":id,"token":token,"body":next,"diagnostics":report}))
    }

    fn take(&self, params: &Value) -> PluginResult<Option<Entry>> {
        let id = params
            .get("runID")
            .and_then(Value::as_str)
            .ok_or_else(invalid)?;
        let token = params
            .get("token")
            .and_then(Value::as_str)
            .ok_or_else(invalid)?;
        if id.len() != 64 || token.len() != 64 {
            return Ok(None);
        }
        let mut runs = self.runs.lock().map_err(|_| unavailable())?;
        if runs
            .get(id)
            .is_some_and(|run| constant_time_equal(&run.token, token))
        {
            return Ok(runs.remove(id));
        }
        Ok(None)
    }

    async fn finish(&self, params: Value, mut run: Entry) -> PluginResult<Value> {
        if !self.completion {
            return Ok(Value::Null);
        }
        let request = params.get("request").cloned().unwrap_or(run.request);
        if !request.is_object() || !fits(&request, MAX_BODY_BYTES) {
            return Err(invalid());
        }
        let reply = params
            .get("reply")
            .filter(|reply| !reply.is_null())
            .cloned();
        if reply
            .as_ref()
            .is_some_and(|reply| !reply.is_object() || !fits(reply, MAX_BODY_BYTES))
        {
            return Err(invalid());
        }
        let completion = Completion {
            context: run.context,
            request,
            reply,
            success: params.get("success").and_then(Value::as_bool) == Some(true),
        };
        tokio::time::timeout(
            run.timeout,
            self.hook
                .on_completion(&completion, &mut run.state, &run.options),
        )
        .await
        .map_err(|_| PluginError::new("timeout", "The gateway completion hook timed out."))?
        .map_err(|_| failed())?;
        Ok(Value::Null)
    }
}

pub fn validate_change(before: &Value, after: &Value) -> PluginResult<()> {
    if !after.is_object() || !fits(after, MAX_BODY_BYTES) {
        return Err(invalid());
    }
    for key in [
        "tools",
        "tool_choice",
        "function_call",
        "functions",
        "toolConfig",
        "model",
        "stream",
    ] {
        if before.get(key) != after.get(key) {
            return Err(invalid());
        }
    }
    for key in [
        "/config/tools",
        "/config/toolConfig",
        "/context/tools",
        "/context/toolSpecs",
        "/options/toolChoice",
        "/options/parallelToolCalls",
        "/reason",
    ] {
        if before.pointer(key) != after.pointer(key) {
            return Err(invalid());
        }
    }
    Ok(())
}

pub fn fits(value: &Value, limit: usize) -> bool {
    struct Limited {
        size: usize,
        limit: usize,
    }
    impl std::io::Write for Limited {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.size = self.size.saturating_add(bytes.len());
            if self.size > self.limit {
                return Err(std::io::Error::other("too large"));
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Limited { size: 0, limit }, value).is_ok()
}

fn timeout(params: &Value) -> PluginResult<Duration> {
    let milliseconds = params
        .get("timeoutMs")
        .map(|value| value.as_u64().ok_or_else(invalid))
        .transpose()?
        .unwrap_or(1000);
    if !(100..=10_000).contains(&milliseconds) {
        return Err(invalid());
    }
    Ok(Duration::from_millis(milliseconds))
}
fn random_id() -> PluginResult<String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|_| unavailable())?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
fn constant_time_equal(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .bytes()
            .zip(right.bytes())
            .fold(0u8, |bits, (a, b)| bits | (a ^ b))
            == 0
}
fn invalid() -> PluginError {
    PluginError::invalid("The gateway hook request is invalid.")
}
fn failed() -> PluginError {
    PluginError::new("hook_failed", "The gateway hook failed.")
}
fn unavailable() -> PluginError {
    PluginError::new("unavailable", "Gateway request hooks are unavailable.")
}

#[cfg(test)]
#[path = "../test/gateway_hooks.test.rs"]
mod tests;
