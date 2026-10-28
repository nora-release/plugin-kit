use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Hook {
    completed: Arc<AtomicUsize>,
}
#[async_trait]
impl GatewayHook for Hook {
    async fn on_request(
        &self,
        context: &Context,
        mut body: Value,
        state: &mut Value,
        options: &Value,
    ) -> PluginResult<Option<Value>> {
        assert_eq!(context.origin, "local");
        *state = json!({"private":"fixture-state"});
        match options.get("case").and_then(Value::as_str) {
            Some("error") => return Err(PluginError::invalid("private-token-must-not-escape")),
            Some("timeout") => tokio::time::sleep(Duration::from_secs(60)).await,
            Some("protected") => body["model"] = json!("other"),
            Some("large-state") => *state = json!({"private":"x".repeat(MAX_STATE_BYTES)}),
            _ => body["injected"] = json!(options["text"]),
        }
        Ok(Some(body))
    }
    async fn on_completion(
        &self,
        completion: &Completion,
        state: &mut Value,
        _options: &Value,
    ) -> PluginResult<()> {
        assert_eq!(state["private"], "fixture-state");
        assert!(completion.request.get("injected").is_some());
        self.completed.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
fn params(options: Value) -> Value {
    json!({"operation":"begin","body":{"model":"p/m","stream":true,"tools":[]},
        "context":{"origin":"local","protocol":"openai-responses","model":"p/m","path":"/v1/responses"},"options":options,"timeoutMs":100})
}
fn hooks() -> (GatewayHooks, Arc<AtomicUsize>) {
    let completed = Arc::new(AtomicUsize::new(0));
    (
        GatewayHooks::new(Hook {
            completed: completed.clone(),
        }),
        completed,
    )
}
#[tokio::test]
async fn state_is_private_bound_to_a_token_and_completion_runs_once() {
    let (hooks, completed) = hooks();
    let run = hooks.call(params(json!({"text":"context"}))).await.unwrap();
    assert!(run.get("state").is_none());
    assert_eq!(run["body"]["injected"], "context");
    let wrong =
        json!({"operation":"finish","runID":run["runID"],"token":"0".repeat(64),"success":true});
    assert!(hooks.call(wrong).await.unwrap().is_null());
    assert_eq!(completed.load(Ordering::SeqCst), 0);
    let finish = json!({"operation":"finish","runID":run["runID"],"token":run["token"],"success":true,"reply":{"ok":true}});
    hooks.call(finish.clone()).await.unwrap();
    hooks.call(finish).await.unwrap();
    assert_eq!(completed.load(Ordering::SeqCst), 1);
    assert!(hooks.runs.lock().unwrap().is_empty());
}
#[tokio::test]
async fn failed_invalid_and_timed_out_hooks_keep_no_run_or_private_error() {
    let (hooks, _) = hooks();
    for case in ["error", "protected", "large-state", "timeout"] {
        let error = hooks.call(params(json!({"case":case}))).await.unwrap_err();
        let encoded = serde_json::to_string(&error).unwrap();
        assert!(!encoded.contains("private-token"));
        assert!(!encoded.contains("fixture-state"));
        assert!(hooks.runs.lock().unwrap().is_empty());
    }
}
#[test]
fn every_tool_and_routing_field_is_protected_but_prompt_context_is_mutable() {
    let original = json!({"model":"m","stream":true,"tools":[],"tool_choice":"auto","function_call":{},"functions":[],"toolConfig":{},"config":{"tools":[],"toolConfig":{}}});
    for pointer in [
        "/model",
        "/stream",
        "/tools",
        "/tool_choice",
        "/function_call",
        "/functions",
        "/toolConfig",
        "/config/tools",
        "/config/toolConfig",
    ] {
        let mut body = original.clone();
        *body.pointer_mut(pointer).unwrap() = json!("changed");
        assert!(validate_change(&original, &body).is_err(), "{pointer}");
    }
    let mut next = original.clone();
    next["messages"] = json!([{"role":"system","content":"context"}]);
    assert!(validate_change(&original, &next).is_ok());
    assert!(validate_change(&original, &Value::Null).is_err());
    assert!(validate_change(&original, &json!({"text":"x".repeat(MAX_BODY_BYTES)})).is_err());
}
#[tokio::test]
async fn close_and_expiration_release_capacity_without_completing() {
    let (hooks, completed) = hooks();
    let run = hooks.call(params(json!({"text":"context"}))).await.unwrap();
    hooks
        .call(json!({"operation":"close","runID":run["runID"],"token":run["token"]}))
        .await
        .unwrap();
    assert_eq!(hooks.capacity.available_permits(), MAX_RUNS);
    let run = hooks.call(params(json!({"text":"context"}))).await.unwrap();
    hooks
        .runs
        .lock()
        .unwrap()
        .get_mut(run["runID"].as_str().unwrap())
        .unwrap()
        .created = Instant::now() - RUN_TTL;
    hooks.prune().unwrap();
    assert_eq!(hooks.capacity.available_permits(), MAX_RUNS);
    assert_eq!(completed.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn a_full_run_table_rejects_immediately_without_evicting_existing_state() {
    let (hooks, _) = hooks();
    for _ in 0..MAX_RUNS {
        hooks.call(params(json!({"text":"context"}))).await.unwrap();
    }
    assert_eq!(
        hooks.call(params(json!({}))).await.unwrap_err().code,
        "busy"
    );
    assert_eq!(hooks.runs.lock().unwrap().len(), MAX_RUNS);
}
#[tokio::test]
async fn invalid_context_options_or_timeout_never_calls_the_hook() {
    let (hooks, _) = hooks();
    let original = params(json!({}));
    for (pointer, value) in [
        ("/context/origin", json!("127.0.0.1")),
        ("/context/path", json!("bad\npath")),
        ("/timeoutMs", json!(99)),
        ("/timeoutMs", json!(10001)),
        ("/options", json!([])),
    ] {
        let mut request = original.clone();
        *request.pointer_mut(pointer).unwrap() = value;
        assert!(hooks.call(request).await.is_err());
    }
    assert!(hooks.runs.lock().unwrap().is_empty());
}

struct PolicyHook;
#[async_trait]
impl GatewayHook for PolicyHook {
    async fn on_request(
        &self,
        _context: &Context,
        mut body: Value,
        _state: &mut Value,
        _options: &Value,
    ) -> PluginResult<Option<Value>> {
        assert!(!crate::subscription_accounts_enabled());
        assert!(crate::require_subscription_accounts().is_err());
        body["policyChecked"] = json!(true);
        Ok(Some(body))
    }
}
struct NativePlugin(GatewayHooks);
#[async_trait]
impl crate::ProviderPlugin for NativePlugin {
    fn gateway_hooks(&self) -> Option<&GatewayHooks> {
        Some(&self.0)
    }
}
#[tokio::test]
async fn native_jsonl_dispatch_advertises_real_hooks_and_applies_the_current_policy_without_accounts(
) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let (mut input, server_input) = tokio::io::duplex(1024 * 1024);
    let (server_output, output) = tokio::io::duplex(1024 * 1024);
    let task = tokio::spawn(crate::serve_io(
        Arc::new(NativePlugin(GatewayHooks::new(PolicyHook))),
        server_input,
        server_output,
    ));
    let mut output = BufReader::new(output);
    let mut line = String::new();
    let init = json!({"id":1,"method":"initialize","params":{"apiVersion":1,"pluginID":"hooks","pluginDirectory":"/plugin","dataDirectory":"/data","manifest":{"authMethods":[{"id":"oauth","type":"oauth"}]},"subscriptionAccountsEnabled":false}});
    input
        .write_all(format!("{init}\n").as_bytes())
        .await
        .unwrap();
    output.read_line(&mut line).await.unwrap();
    let result: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        result["result"]["capabilities"]["gatewayHooks"]["request"],
        true
    );
    let mut hook = params(json!({}));
    hook["subscriptionAccountsEnabled"] = json!(false);
    input
        .write_all(format!("{}\n", json!({"id":2,"method":"gatewayHook","params":hook})).as_bytes())
        .await
        .unwrap();
    line.clear();
    output.read_line(&mut line).await.unwrap();
    let result: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(result["result"]["body"]["policyChecked"], true);
    assert!(result["result"].get("state").is_none());
    drop(input);
    task.await.unwrap().unwrap();
}

#[test]
fn native_tool_declarations_and_tool_selection_are_also_protected() {
    let before = json!({"model":{"provider":"p","id":"m","api":"openai-completions"},"context":{"tools":[],"toolSpecs":[]},"options":{"toolChoice":"auto","parallelToolCalls":true}});
    for pointer in [
        "/context/tools",
        "/context/toolSpecs",
        "/options/toolChoice",
        "/options/parallelToolCalls",
    ] {
        let mut after = before.clone();
        *after.pointer_mut(pointer).unwrap() = json!("changed");
        assert!(validate_change(&before, &after).is_err());
    }
}

#[tokio::test]
async fn verified_input_files_allow_large_bodies_but_changed_or_oversized_descriptors_are_rejected()
{
    let path = std::env::temp_dir().join(format!(
        "nora-sdk-hook-body-{}-{}",
        std::process::id(),
        random_id().unwrap()
    ));
    let body = json!({"model":"p/m","stream":true,"tools":[],"text":"x".repeat(700*1024)});
    std::fs::write(&path, serde_json::to_vec(&body).unwrap()).unwrap();
    let descriptor = crate::RequestBodyFile::from_path(&path).unwrap();
    let (hooks, _) = hooks();
    let mut request = params(json!({"text":"context"}));
    request.as_object_mut().unwrap().remove("body");
    request["bodyFile"] = json!(descriptor);
    let run = hooks.call(request.clone()).await.unwrap();
    assert_eq!(run["body"]["text"].as_str().unwrap().len(), 700 * 1024);
    hooks
        .call(json!({"operation":"close","runID":run["runID"],"token":run["token"]}))
        .await
        .unwrap();
    std::fs::write(&path, b"{}").unwrap();
    assert!(hooks.call(request.clone()).await.is_err());
    request["bodyFile"]["length"] = json!(MAX_BODY_BYTES + 1);
    assert!(hooks.call(request).await.is_err());
    std::fs::remove_file(path).unwrap();
}
#[tokio::test]
async fn large_hook_results_stream_in_small_frames_with_an_integrity_checked_final_reference() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use sha2::{Digest, Sha256};
    use tokio::io::{AsyncBufReadExt, BufReader};
    let result = json!({"runID":"a".repeat(64),"token":"b".repeat(64),"body":{"model":"m","text":"x".repeat(700*1024)}});
    let expected = result["body"].clone();
    let (output, input) = tokio::io::duplex(1024 * 1024);
    let writer = Arc::new(tokio::sync::Mutex::new(output));
    let task = tokio::spawn(async move {
        transport::write_result(&writer, 3, result).await.unwrap();
    });
    let mut reader = BufReader::new(input);
    let mut bytes = Vec::new();
    let mut sequence = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        assert!(line.len() < crate::MAX_RESPONSE_FRAME_BYTES);
        let frame: Value = serde_json::from_str(&line).unwrap();
        if let Some(result) = frame.get("result") {
            assert!(result.get("body").is_none());
            assert_eq!(result["bodyStream"]["length"], bytes.len());
            assert_eq!(
                result["bodyStream"]["sha256"],
                format!("{:x}", Sha256::digest(&bytes))
            );
            break;
        }
        assert_eq!(frame["event"], "gatewayHookBody");
        assert_eq!(frame["sequence"], sequence);
        sequence += 1;
        let decoded = STANDARD.decode(frame["data"].as_str().unwrap()).unwrap();
        assert!(decoded.len() <= crate::MAX_HTTP_CHUNK_BYTES);
        bytes.extend(decoded);
    }
    assert!(sequence > 1);
    assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), expected);
    task.await.unwrap();
}

#[tokio::test]
async fn wrong_tokens_cannot_trigger_completion_file_reads() {
    let (hooks, completed) = hooks();
    let run = hooks.call(params(json!({"text":"context"}))).await.unwrap();
    let request = json!({"operation":"finish","runID":run["runID"],"token":"f".repeat(64),"replyFile":{"path":"/never-read","length":MAX_BODY_BYTES+1,"sha256":"0".repeat(64)}});
    assert!(hooks.call(request).await.unwrap().is_null());
    assert_eq!(completed.load(Ordering::SeqCst), 0);
    hooks
        .call(json!({"operation":"close","runID":run["runID"],"token":run["token"]}))
        .await
        .unwrap();
}
struct ReportingHook;
#[async_trait]
impl GatewayHook for ReportingHook {
    async fn on_request(
        &self,
        context: &Context,
        _body: Value,
        _state: &mut Value,
        _options: &Value,
    ) -> PluginResult<Option<Value>> {
        context.report(Outcome::Unbound, 9999);
        Ok(None)
    }
}
#[tokio::test]
async fn diagnostic_reports_contain_only_typed_outcomes_and_bounded_counts() {
    let hooks = GatewayHooks::new(ReportingHook);
    let run = hooks.call(params(json!({}))).await.unwrap();
    assert_eq!(run["diagnostics"], json!({"outcome":"unbound","bytes":0}));
    hooks
        .call(json!({"operation":"close","runID":run["runID"],"token":run["token"]}))
        .await
        .unwrap();
    let context = Context::default();
    context.report(Outcome::Ready, 32769);
    assert_eq!(context.report_state.0.lock().unwrap().bytes, 0);
    assert!(serde_json::to_value(context)
        .unwrap()
        .get("report_state")
        .is_none());
}
