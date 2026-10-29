use crate::{PluginError, PluginResult};
use serde_json::Value;

tokio::task_local! { static ACCOUNTS_ENABLED: bool; }

/// Each function takes a package's `contributes.provider` declaration, which
/// owns authentication categories.
///
/// A source binding wins
/// over a token's storage shape: several CLI sign-ins happen to store API keys.
pub fn subscription_account(manifest: &Value, credentials: &Value) -> bool {
    if credentials
        .get("source")
        .is_some_and(|value| !value.is_null())
        || ["noraAccountSource", "noraClaudeCLI"].iter().any(|key| {
            credentials
                .get("extras")
                .and_then(|extras| extras.get(*key))
                .is_some_and(|value| !value.is_null())
        })
    {
        return true;
    }
    let kind = credentials
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if matches!(
        kind,
        "oauth" | "source" | "codeAssistSource" | "claudeCode" | "pendingImport"
    ) {
        return true;
    }
    for key in ["authMethodID", "authMethod"] {
        if let Some(method) = credentials.get(key).and_then(Value::as_str) {
            if let Some(declaration) = method_of(manifest, method) {
                return method_subscription(manifest, declaration);
            }
            if matches!(method, "oauth" | "source") {
                return true;
            }
        }
    }
    if matches!(kind, "api" | "apiKey")
        || credentials.get("key").is_some_and(Value::is_string)
        || credentials
            .get("authMethod")
            .and_then(Value::as_str)
            .is_some_and(|method| matches!(method, "api" | "apiKey"))
    {
        return false;
    }
    has_subscription_methods(manifest)
}

pub fn subscription_method(manifest: &Value, method_id: &str) -> bool {
    method_of(manifest, method_id)
        .map(|method| method_subscription(manifest, method))
        .unwrap_or_else(|| {
            matches!(method_id, "oauth" | "source") || has_subscription_methods(manifest)
        })
}

pub fn has_subscription_methods(manifest: &Value) -> bool {
    methods(manifest).any(|method| method_subscription(manifest, method))
        || manifest
            .pointer("/config/sourceAuthMethods")
            .and_then(Value::as_array)
            .is_some_and(|values| !values.is_empty())
        || manifest
            .get("localSignIns")
            .and_then(Value::as_array)
            .is_some_and(|values| !values.is_empty())
}

/// A single classifier is shared by the host and the child process. Neither
/// account labels nor provider IDs determine whether credentials are allowed.
pub fn request_uses_subscription(manifest: &Value, method: &str, params: &Value) -> bool {
    match method {
        "initialize"
        | "authMethods"
        | "modelMetadata"
        | "gatewayHook"
        | "abort"
        | "credentialsSaved"
        | "httpRequestAllowed"
        | "cancelAuthorization" => false,
        // Signing out must remain possible when subscriptions are disabled.
        "revokeAccount" => false,
        "discoverAccounts" | "consumeReset" => true,
        "inspectAccountImport" | "importAccount" => {
            if !matches!(
                params.get("format").and_then(Value::as_str),
                Some("apiKeys" | "api-keys")
            ) {
                return true;
            }
            let selected = selected_method(params)
                .and_then(|id| method_of(manifest, id))
                .or_else(|| {
                    if selected_method(params).is_none() {
                        methods(manifest).find(|method| {
                            matches!(
                                method.get("type").and_then(Value::as_str),
                                Some("api" | "apiKey")
                            ) && !method_subscription(manifest, method)
                        })
                    } else {
                        None
                    }
                });
            selected.is_none_or(|method| {
                method_subscription(manifest, method)
                    || !matches!(
                        method.get("type").and_then(Value::as_str),
                        Some("api" | "apiKey")
                    )
            })
        }
        "authorize" | "callback" => selected_method(params)
            .map(|id| subscription_method(manifest, id))
            .unwrap_or_else(|| has_subscription_methods(manifest)),
        "maintain" => has_subscription_methods(manifest),
        _ => {
            if let Some(credentials) = params.get("credentials").filter(|value| !value.is_null()) {
                return subscription_account(manifest, credentials);
            }
            if method == "models" {
                return has_subscription_methods(manifest)
                    && !methods(manifest).any(|method| !method_subscription(manifest, method));
            }
            has_subscription_methods(manifest)
        }
    }
}

fn selected_method(params: &Value) -> Option<&str> {
    ["authMethodID", "method", "authMethod"]
        .into_iter()
        .find_map(|key| {
            params
                .get(key)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
        })
}
fn methods(manifest: &Value) -> impl Iterator<Item = &Value> {
    [
        manifest.get("authMethods"),
        manifest.pointer("/config/sourceAuthMethods"),
    ]
    .into_iter()
    .flatten()
    .filter_map(Value::as_array)
    .flatten()
}
fn method_of<'a>(manifest: &'a Value, id: &str) -> Option<&'a Value> {
    methods(manifest)
        .find(|method| {
            method.get("id").and_then(Value::as_str) == Some(id) || method.as_str() == Some(id)
        })
        .or_else(|| {
            manifest
                .get("localSignIns")
                .and_then(Value::as_array)?
                .iter()
                .find(|source| source.get("method").and_then(Value::as_str) == Some(id))
        })
}
fn subscription_declaration(method: &Value) -> bool {
    if method.get("purpose").and_then(Value::as_str) == Some("subscription") {
        return true;
    }
    !matches!(
        method.get("type").and_then(Value::as_str),
        Some("api" | "apiKey" | "none" | "keyless")
    )
}
fn method_subscription(manifest: &Value, method: &Value) -> bool {
    let id = method
        .get("id")
        .or_else(|| method.get("method"))
        .and_then(Value::as_str)
        .or_else(|| method.as_str());
    id.is_some_and(|id| source_method(manifest, id)) || subscription_declaration(method)
}
fn source_method(manifest: &Value, id: &str) -> bool {
    manifest
        .pointer("/config/sourceAuthMethods")
        .and_then(Value::as_array)
        .is_some_and(|methods| {
            methods.iter().any(|method| {
                method.as_str() == Some(id) || method.get("id").and_then(Value::as_str) == Some(id)
            })
        })
        || manifest
            .get("localSignIns")
            .and_then(Value::as_array)
            .is_some_and(|sources| {
                sources
                    .iter()
                    .any(|source| source.get("method").and_then(Value::as_str) == Some(id))
            })
}

/// Task-local policy covers direct provider helpers as well as dispatch. The
/// environment protects work outside a request, including initialization.
pub fn subscription_accounts_enabled() -> bool {
    !std::env::var("NORA_NO_ACCOUNTS")
        .ok()
        .is_some_and(|value| no_accounts_value(&value))
        && ACCOUNTS_ENABLED
            .try_with(|enabled| *enabled)
            .unwrap_or(true)
}
pub fn no_accounts_value(value: &str) -> bool {
    ["1", "true", "on"]
        .iter()
        .any(|truthy| value.trim().eq_ignore_ascii_case(truthy))
}
pub fn require_subscription_accounts() -> PluginResult<()> {
    if subscription_accounts_enabled() {
        return Ok(());
    }
    Err(disabled())
}
pub(crate) fn disabled() -> PluginError {
    PluginError::new(
        "subscription_accounts_disabled",
        "Subscription accounts are disabled in Nora settings.",
    )
}
pub(crate) async fn scope<T>(enabled: bool, future: impl std::future::Future<Output = T>) -> T {
    ACCOUNTS_ENABLED.scope(enabled, future).await
}

#[cfg(test)]
#[path = "../test/subscription_policy.test.rs"]
mod tests;
