//! Provider plugins: the service `ai.provider`, which the router asks about
//! a model provider's accounts (Nora's docs/provider-plugins.md). A plugin
//! implements [`Provider`] and runs [`serve_provider`]; a question it
//! doesn't implement is answered as unsupported.

mod types;

pub use types::*;

use crate::protocol::{
    CallToolParams, InitializeParams, InitializeResult, RpcError, ServiceCallParams, ToolResult,
};
use crate::{Plugin, PluginContext};
use serde::Serialize;
use serde_json::{json, Value};
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The service a provider plugin provides.
pub const SERVICE: &str = "ai.provider";

/// The account must be signed in to again.
pub const SIGN_IN_NEEDED: i32 = -32010;
/// The vendor turned the account away for now.
pub const RATE_LIMITED: i32 = -32011;

/// The error that tells the router the account must be signed in to again.
pub fn sign_in_needed(reason: impl Into<String>) -> RpcError {
    RpcError::new(SIGN_IN_NEEDED, reason)
}

/// The error that tells the router the vendor turned the account away;
/// it rests for `retry_after`, a minute when unknown.
pub fn rate_limited(reason: impl Into<String>, retry_after: Option<Duration>) -> RpcError {
    let mut error = RpcError::new(RATE_LIMITED, reason);
    if let Some(wait) = retry_after {
        error.data = Some(json!({"retryAfter": wait.as_secs_f64()}));
    }
    error
}

/// `time`, as RFC 3339 in UTC, for an account's `expires_at`.
pub fn rfc3339(time: SystemTime) -> String {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let (days, rest) = (seconds / 86_400, seconds % 86_400);
    // civil date from days since 1970-01-01 (Howard Hinnant's algorithm)
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// The questions the router asks a provider plugin. Each defaults to
/// unsupported; a plugin implements those its manifest says it answers.
pub trait Provider: Send + Sync + 'static {
    /// `provider/signin`.
    fn sign_in(&self, _request: SignIn) -> impl Future<Output = Result<Step, RpcError>> + Send {
        unsupported()
    }

    /// `provider/signin/finish`.
    fn sign_in_finish(
        &self,
        _request: SignInFinish,
    ) -> impl Future<Output = Result<Step, RpcError>> + Send {
        unsupported()
    }

    /// `provider/refresh`: renew credentials about to expire.
    fn refresh(
        &self,
        _question: Question,
    ) -> impl Future<Output = Result<Renewed, RpcError>> + Send {
        unsupported()
    }

    /// `provider/models`: the models the account can use.
    fn models(
        &self,
        _question: Question,
    ) -> impl Future<Output = Result<Vec<Model>, RpcError>> + Send {
        unsupported()
    }

    /// `provider/connect`: where the router sends the account's requests.
    fn connect(
        &self,
        _question: Question,
    ) -> impl Future<Output = Result<Connection, RpcError>> + Send {
        unsupported()
    }

    /// `provider/fetch`: send one request, for a provider whose manifest
    /// says `sendsRequests`.
    fn fetch(&self, _request: Fetch) -> impl Future<Output = Result<FetchHead, RpcError>> + Send {
        unsupported()
    }

    /// `provider/usage`.
    fn usage(&self, _question: Question) -> impl Future<Output = Result<Usage, RpcError>> + Send {
        unsupported()
    }

    /// `provider/checkin`.
    fn checkin(
        &self,
        _question: Question,
    ) -> impl Future<Output = Result<Checkin, RpcError>> + Send {
        unsupported()
    }
}

fn unsupported<T>() -> std::future::Ready<Result<T, RpcError>> {
    std::future::ready(Err(RpcError::method_not_found()))
}

/// Serves `provider` on stdin and stdout until the router closes them.
pub async fn serve_provider(provider: impl Provider) -> std::io::Result<()> {
    crate::serve(ProviderPlugin(Arc::new(provider))).await
}

/// A [`Provider`] as a plugin providing [`SERVICE`].
pub struct ProviderPlugin<P>(pub Arc<P>);

impl<P: Provider> Plugin for ProviderPlugin<P> {
    async fn initialize(
        &self,
        _params: InitializeParams,
        _context: PluginContext,
    ) -> Result<InitializeResult, RpcError> {
        Ok(InitializeResult {
            services: vec![SERVICE.into()],
            ..InitializeResult::default()
        })
    }

    async fn call_tool(
        &self,
        _params: CallToolParams,
        _context: PluginContext,
    ) -> Result<ToolResult, RpcError> {
        Err(RpcError::method_not_found())
    }

    async fn call_service(
        &self,
        params: ServiceCallParams,
        context: PluginContext,
    ) -> Result<Value, RpcError> {
        if params.service != SERVICE {
            return Err(RpcError::method_not_found());
        }
        let provider = &self.0;
        let arguments = params.arguments;
        match params.method.as_str() {
            "provider/signin" => answer(provider.sign_in(read(arguments)?).await),
            "provider/signin/finish" => answer(provider.sign_in_finish(read(arguments)?).await),
            "provider/refresh" => answer(provider.refresh(read(arguments)?).await),
            "provider/models" => answer(provider.models(read(arguments)?).await),
            "provider/connect" => answer(provider.connect(read(arguments)?).await),
            "provider/fetch" => answer(provider.fetch(fetch(arguments, &context)?).await),
            "provider/usage" => answer(provider.usage(read(arguments)?).await),
            "provider/checkin" => answer(provider.checkin(read(arguments)?).await),
            _ => Err(RpcError::method_not_found()),
        }
    }
}

fn read<T: serde::de::DeserializeOwned>(arguments: Value) -> Result<T, RpcError> {
    serde_json::from_value(arguments)
        .map_err(|error| RpcError::invalid_params(format!("Unreadable question: {error}")))
}

fn answer<T: Serialize>(result: Result<T, RpcError>) -> Result<Value, RpcError> {
    serde_json::to_value(result?).map_err(|error| RpcError::new(-32603, error.to_string()))
}

fn fetch(arguments: Value, context: &PluginContext) -> Result<Fetch, RpcError> {
    #[derive(serde::Deserialize)]
    struct Request {
        method: String,
        url: String,
        #[serde(default)]
        headers: std::collections::BTreeMap<String, String>,
    }
    #[derive(serde::Deserialize)]
    struct Arguments {
        #[serde(flatten)]
        question: Question,
        request: Request,
    }
    let Arguments { question, request } = read(arguments)?;
    let streams = || RpcError::invalid_params("provider/fetch needs protocol 2's streams");
    Ok(Fetch {
        question,
        method: request.method,
        url: request.url,
        headers: request.headers,
        body: context.take_input_stream().ok_or_else(streams)?,
        response: context.take_output_stream().ok_or_else(streams)?,
    })
}

#[cfg(test)]
#[path = "../../test/provider.test.rs"]
mod tests;
