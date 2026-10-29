use super::*;
use crate::{serve_io, CallContext, FetchRequest, FetchResponse, InitializeParams, ProviderPlugin};
use serde_json::json;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};

fn mixed() -> Value {
    json!({"authMethods":[{"id":"oauth","type":"oauth"},{"id":"apiKey","type":"api"},{"id":"cli","type":"api","purpose":"subscription"}],"localSignIns":[{"method":"local"}],"config":{"sourceAuthMethods":[{"id":"editor","type":"source"}]}})
}
#[test]
fn environment_flags_and_canonical_api_key_import_formats_match_the_host() {
    for value in ["1", "true", "on", "TRUE", "ON", " on "] {
        assert!(no_accounts_value(value));
    }
    for value in ["0", "false", "off", "", "yes", "2"] {
        assert!(!no_accounts_value(value));
    }
    for format in ["apiKeys", "api-keys"] {
        assert!(!request_uses_subscription(
            &mixed(),
            "importAccount",
            &json!({"format":format,"method":"apiKey"})
        ));
        assert!(request_uses_subscription(
            &mixed(),
            "importAccount",
            &json!({"format":format,"method":"oauth"})
        ));
    }
    for method in [
        "abort",
        "credentialsSaved",
        "httpRequestAllowed",
        "cancelAuthorization",
        "modelMetadata",
        "revokeAccount",
    ] {
        assert!(!request_uses_subscription(&mixed(), method, &json!({})));
    }
}
#[test]
fn legacy_source_method_strings_override_api_key_storage_shape() {
    let manifest = json!({"authMethods":[{"id":"apiKey","type":"api"}],"config":{"sourceAuthMethods":["sourceCLI"]}});
    assert!(has_subscription_methods(&manifest));
    assert!(subscription_method(&manifest, "sourceCLI"));
    assert!(subscription_account(
        &manifest,
        &json!({"type":"apiKey","authMethod":"sourceCLI"})
    ));
    assert!(!subscription_account(
        &manifest,
        &json!({"type":"apiKey","authMethod":"apiKey"})
    ));
    let local = json!({"authMethods":[{"id":"apiKey","type":"api"}],"localSignIns":[{"method":"factoryCLI"}]});
    assert!(subscription_account(
        &local,
        &json!({"type":"apiKey","authMethod":"factoryCLI"})
    ));
    assert!(!subscription_account(
        &local,
        &json!({"type":"apiKey","authMethod":"apiKey"})
    ));
}
#[test]
fn local_source_roles_override_legacy_api_method_type_before_login() {
    for source in [
        json!({"localSignIns":[{"method":"cliSource"}]}),
        json!({"config":{"sourceAuthMethods":[{"id":"cliSource","type":"api"}]}}),
    ] {
        let mut manifest =
            json!({"authMethods":[{"id":"cliSource","type":"api"},{"id":"apiKey","type":"api"}]});
        manifest
            .as_object_mut()
            .unwrap()
            .extend(source.as_object().unwrap().clone());
        assert!(has_subscription_methods(&manifest));
        assert!(subscription_method(&manifest, "cliSource"));
        assert!(!subscription_method(&manifest, "apiKey"));
        assert!(subscription_account(
            &manifest,
            &json!({"type":"apiKey","authMethod":"cliSource"})
        ));
        assert!(request_uses_subscription(
            &manifest,
            "authorize",
            &json!({"method":"cliSource"})
        ));
        assert!(!request_uses_subscription(
            &manifest,
            "authorize",
            &json!({"method":"apiKey"})
        ));
        assert!(!request_uses_subscription(
            &manifest,
            "importAccount",
            &json!({"format":"apiKeys"})
        ));
        assert!(request_uses_subscription(
            &manifest,
            "importAccount",
            &json!({"format":"apiKeys","method":"cliSource"})
        ));
    }
}
#[test]
fn method_and_account_categories_are_defined_by_authentication_not_provider_names() {
    let manifest = mixed();
    assert!(has_subscription_methods(&manifest));
    for id in ["oauth", "cli", "editor", "local", "unknown"] {
        assert!(subscription_method(&manifest, id), "{id}");
    }
    assert!(!subscription_method(&manifest, "apiKey"));
    for credentials in [
        json!({"type":"oauth","authMethod":"apiKey"}),
        json!({"type":"apiKey","authMethod":"cli"}),
        json!({"type":"apiKey","source":{"path":"private"}}),
        json!({"type":"apiKey","extras":{"noraAccountSource":{"kind":"cli"}}}),
        json!({"type":"apiKey","extras":{"noraClaudeCLI":{"directory":"private"}}}),
        json!({"type":"source"}),
        json!({"type":"pendingImport"}),
        json!({"access":"opaque"}),
    ] {
        assert!(
            subscription_account(&manifest, &credentials),
            "{credentials}"
        );
    }
    for credentials in [
        json!({"type":"api"}),
        json!({"type":"apiKey"}),
        json!({"key":"private"}),
        json!({"authMethod":"apiKey"}),
    ] {
        assert!(
            !subscription_account(&manifest, &credentials),
            "{credentials}"
        );
    }
    let api = json!({"id":"subscription-looking-name","authMethods":[{"id":"aws","type":"api"}]});
    assert!(!has_subscription_methods(&api));
    assert!(!subscription_account(&api, &json!({"type":"aws"})));
}
#[test]
fn api_imports_callbacks_and_keyless_models_are_separate_from_subscription_operations() {
    let manifest = mixed();
    for method in [
        "discoverAccounts",
        "maintain",
        "consumeReset",
        "inspectAccountImport",
        "importAccount",
        "authorize",
        "callback",
        "usage",
        "refresh",
        "fetch",
        "accounts",
    ] {
        assert!(
            request_uses_subscription(&manifest, method, &json!({})),
            "{method}"
        );
    }
    for method in ["usage", "refresh", "fetch", "accounts", "models"] {
        assert!(
            !request_uses_subscription(
                &manifest,
                method,
                &json!({"credentials":{"type":"apiKey"}})
            ),
            "{method}"
        );
        assert!(
            request_uses_subscription(&manifest, method, &json!({"credentials":{"type":"oauth"}})),
            "{method}"
        );
    }
    for method in ["inspectAccountImport", "importAccount"] {
        assert!(!request_uses_subscription(
            &manifest,
            method,
            &json!({"format":"api-keys","method":"apiKey"})
        ));
        assert!(!request_uses_subscription(
            &manifest,
            method,
            &json!({"format":"api-keys"})
        ));
        for selected in ["oauth", "cli", "unknown"] {
            assert!(request_uses_subscription(
                &manifest,
                method,
                &json!({"format":"api-keys","method":selected})
            ));
        }
    }
    assert!(!request_uses_subscription(
        &manifest,
        "callback",
        &json!({"method":"apiKey"})
    ));
    assert!(!request_uses_subscription(
        &manifest,
        "authorize",
        &json!({"authMethodID":"apiKey"})
    ));
    assert!(!request_uses_subscription(&manifest, "models", &json!({})));
    let subscription = json!({"authMethods":[{"id":"oauth","type":"oauth"}]});
    assert!(request_uses_subscription(
        &subscription,
        "models",
        &json!({})
    ));
    let keyless = json!({"authMethods":[]});
    assert!(!request_uses_subscription(&keyless, "models", &json!({})));
    assert!(!request_uses_subscription(&keyless, "maintain", &json!({})));
    assert!(request_uses_subscription(
        &keyless,
        "importAccount",
        &json!({"format":"api-keys"})
    ));
}
#[tokio::test]
async fn task_local_policy_does_not_leak_between_parallel_requests() {
    let disabled = scope(false, async {
        tokio::task::yield_now().await;
        require_subscription_accounts().unwrap_err().code
    });
    let enabled = scope(true, async {
        tokio::task::yield_now().await;
        require_subscription_accounts().is_ok()
    });
    let (code, allowed) = tokio::join!(disabled, enabled);
    assert_eq!(code, "subscription_accounts_disabled");
    assert!(allowed);
    assert!(subscription_accounts_enabled());
}
#[test]
fn environment_guard_child() {
    if std::env::var_os("NORA_POLICY_CHILD").is_none() {
        return;
    }
    assert!(!subscription_accounts_enabled());
    assert_eq!(
        require_subscription_accounts().unwrap_err().code,
        "subscription_accounts_disabled"
    );
}
#[test]
fn process_environment_protects_work_outside_requests() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "subscription_policy::tests::environment_guard_child",
        ])
        .env("NORA_POLICY_CHILD", "1")
        .env("NORA_NO_ACCOUNTS", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[derive(Default)]
struct Plugin {
    calls: Mutex<Vec<String>>,
}
#[async_trait::async_trait]
impl ProviderPlugin for Plugin {
    async fn initialize(&self, _: InitializeParams) -> PluginResult<Value> {
        self.calls.lock().unwrap().push("initialize".into());
        Ok(json!({"enabled":subscription_accounts_enabled()}))
    }
    async fn models(&self, _: CallContext) -> PluginResult<Value> {
        self.calls.lock().unwrap().push("models".into());
        Ok(json!({"enabled":subscription_accounts_enabled()}))
    }
    async fn accounts(&self, _: CallContext) -> PluginResult<Value> {
        self.calls.lock().unwrap().push("accounts".into());
        Ok(json!({"ok":true}))
    }
    async fn discover_accounts(&self, _: CallContext) -> PluginResult<Value> {
        self.calls.lock().unwrap().push("discoverAccounts".into());
        Ok(json!({"ok":true}))
    }
    async fn authorize(&self, _: Value) -> PluginResult<Value> {
        self.calls.lock().unwrap().push("authorize".into());
        Ok(json!({"ok":true}))
    }
    async fn callback(&self, _: Value) -> PluginResult<Value> {
        self.calls.lock().unwrap().push("callback".into());
        Ok(json!({"ok":true}))
    }
    async fn refresh(&self, _: Value) -> PluginResult<Value> {
        self.calls.lock().unwrap().push("refresh".into());
        Ok(json!({"ok":true}))
    }
    async fn usage(&self, _: CallContext) -> PluginResult<Value> {
        self.calls.lock().unwrap().push("usage".into());
        Ok(json!({"ok":true}))
    }
    async fn fetch(&self, request: FetchRequest) -> PluginResult<FetchResponse> {
        self.calls.lock().unwrap().push("fetch".into());
        if request.url == "source-body" {
            return Ok(FetchResponse {
                status: 200,
                headers: Default::default(),
                body: Box::pin(futures::stream::once(async {
                    require_subscription_accounts()?;
                    Ok(b"private-source".to_vec())
                })),
            });
        }
        Ok(FetchResponse::bytes(200, Default::default(), vec![]))
    }
    async fn call(&self, method: &str, _: Value) -> PluginResult<Value> {
        self.calls.lock().unwrap().push(method.into());
        if method == "directSourceHelper" {
            require_subscription_accounts()?;
        }
        Ok(json!({"ok":true}))
    }
}
struct Harness {
    input: WriteHalf<DuplexStream>,
    output: BufReader<ReadHalf<DuplexStream>>,
    task: tokio::task::JoinHandle<std::io::Result<()>>,
    plugin: Arc<Plugin>,
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Harness {
    fn new() -> Self {
        let (client, server) = tokio::io::duplex(1024 * 1024);
        let (input, output) = tokio::io::split(server);
        let (read, write) = tokio::io::split(client);
        let plugin = Arc::new(Plugin::default());
        let task = tokio::spawn(serve_io(plugin.clone(), input, output));
        Self {
            input: write,
            output: BufReader::new(read),
            task,
            plugin,
        }
    }
    async fn call(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.input
            .write_all(format!("{}\n", json!({"id":id,"method":method,"params":params})).as_bytes())
            .await
            .unwrap();
        let mut line = String::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            self.output.read_line(&mut line),
        )
        .await
        .unwrap()
        .unwrap();
        serde_json::from_str(&line).unwrap()
    }
    async fn initialize(&mut self, enabled: Option<bool>) {
        let mut params = json!({"apiVersion":2,"pluginID":"fixture","pluginDirectory":"/plugin","dataDirectory":"/private","manifest":{"contributes":{"provider":mixed()}}});
        if let Some(enabled) = enabled {
            params["subscriptionAccountsEnabled"] = json!(enabled);
        }
        assert!(self.call(1, "initialize", params).await["error"].is_null());
    }
}
#[tokio::test]
async fn disabled_subscription_methods_never_reach_provider_implementations() {
    let mut harness = Harness::new();
    harness.initialize(Some(false)).await;
    for (index, method) in [
        "models",
        "accounts",
        "usage",
        "refresh",
        "fetch",
        "discoverAccounts",
        "authorize",
        "callback",
        "maintain",
        "importAccount",
        "inspectAccountImport",
        "consumeReset",
    ]
    .into_iter()
    .enumerate()
    {
        let params = json!({"credentials":{"type":"oauth"},"method":"oauth","url":"https://private.invalid","body":null});
        let response = harness.call(index as u64 + 2, method, params).await;
        assert_eq!(
            response["error"]["code"], "subscription_accounts_disabled",
            "{method}: {response}"
        );
    }
    assert_eq!(*harness.plugin.calls.lock().unwrap(), ["initialize"]);
}
#[tokio::test]
async fn api_methods_still_work_and_source_helpers_cannot_be_used_from_an_api_request() {
    let mut harness = Harness::new();
    harness.initialize(Some(false)).await;
    for (index, method) in [
        "models",
        "accounts",
        "usage",
        "refresh",
        "authorize",
        "callback",
        "importAccount",
    ]
    .into_iter()
    .enumerate()
    {
        let response = harness
            .call(
                index as u64 + 2,
                method,
                json!({"credentials":{"type":"apiKey"},"method":"apiKey","format":"api-keys"}),
            )
            .await;
        assert!(response["error"].is_null(), "{response}");
    }
    let response = harness
        .call(
            40,
            "directSourceHelper",
            json!({"credentials":{"type":"apiKey"}}),
        )
        .await;
    assert_eq!(response["error"]["code"], "subscription_accounts_disabled");
    let head=harness.call(41,"fetch",json!({"credentials":{"type":"apiKey"},"url":"https://api.invalid","method":"GET","body":null})).await;
    assert_eq!(head["status"], 200);
    let mut done = String::new();
    harness.output.read_line(&mut done).await.unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&done).unwrap()["event"],
        "done"
    );
}
#[tokio::test]
async fn request_flags_override_initial_state_without_mixing_concurrent_authentication_types() {
    let mut harness = Harness::new();
    harness.initialize(None).await;
    let denied = harness
        .call(
            2,
            "usage",
            json!({"subscriptionAccountsEnabled":false,"credentials":{"type":"oauth"}}),
        )
        .await;
    assert_eq!(denied["error"]["code"], "subscription_accounts_disabled");
    let allowed = harness
        .call(
            3,
            "usage",
            json!({"subscriptionAccountsEnabled":true,"credentials":{"type":"oauth"}}),
        )
        .await;
    assert!(allowed["error"].is_null());
    let invalid = harness
        .call(
            4,
            "models",
            json!({"subscriptionAccountsEnabled":"private-invalid"}),
        )
        .await;
    assert_eq!(invalid["error"]["code"], "invalid_request");
    assert!(!invalid.to_string().contains("private-invalid"));
}
#[tokio::test]
async fn source_package_manifest_owns_policy_even_if_effective_manifest_is_overridden() {
    let mut harness = Harness::new();
    let params = json!({"apiVersion":2,"pluginID":"fixture","pluginDirectory":"/plugin","dataDirectory":"/private","subscriptionAccountsEnabled":false,"manifest":{"contributes":{"provider":{"authMethods":[{"id":"oauth","type":"api"}]}}},"installedManifest":{"contributes":{"provider":mixed()}}});
    let initialized = harness.call(1, "initialize", params).await;
    assert_eq!(initialized["result"]["enabled"], false);
    let response = harness
        .call(2, "authorize", json!({"method":"oauth"}))
        .await;
    assert_eq!(response["error"]["code"], "subscription_accounts_disabled");
}
#[tokio::test]
async fn policy_remains_active_while_polling_streaming_response_bodies() {
    let mut harness = Harness::new();
    harness.initialize(None).await;
    let head=harness.call(2,"fetch",json!({"subscriptionAccountsEnabled":false,"credentials":{"type":"apiKey"},"url":"source-body","method":"GET","body":null})).await;
    assert_eq!(head["status"], 200);
    let mut line = String::new();
    harness.output.read_line(&mut line).await.unwrap();
    let response: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["error"]["code"], "subscription_accounts_disabled");
    assert!(!line.contains("private-source"));
}
