use async_trait::async_trait;
use nora_provider_plugin_sdk::{
    gateway_hooks::{Completion, Context, GatewayHook, GatewayHooks},
    PluginResult, ProviderPlugin,
};
use serde_json::{json, Value};
use std::io::Write;

// This native test child stores lengths only. It must never become an installed
// provider or retain prompts, recalled text, credentials or model replies.
struct Hook;
fn record(value: Value) {
    let directory = std::env::var_os("NORA_PROVIDER_DATA_DIR").unwrap();
    let path = std::path::Path::new(&directory).join("hook-proof");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    writeln!(file, "{value}").unwrap();
}
#[async_trait]
impl GatewayHook for Hook {
    async fn on_request(
        &self,
        context: &Context,
        mut body: Value,
        state: &mut Value,
        _options: &Value,
    ) -> PluginResult<Option<Value>> {
        record(json!({"phase":"begin","requestBytes":serde_json::to_vec(&body).unwrap().len()}));
        *state = json!({"ownRun":true});
        if context.protocol == "nora" {
            let prompt = body["context"]["systemPrompt"].as_str().unwrap_or_default();
            body["context"]["systemPrompt"] =
                json!(format!("{prompt} compiled-native-hook-context"));
            body["options"]["metadata"]["compiledHook"] = json!(true);
        } else {
            if let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) {
                messages.insert(
                    0,
                    json!({"role":"system","content":"compiled-native-hook-context"}),
                );
            }
            body["compiledHook"] = json!(true);
        }
        Ok(Some(body))
    }
    async fn on_completion(
        &self,
        completion: &Completion,
        state: &mut Value,
        _options: &Value,
    ) -> PluginResult<()> {
        assert_eq!(state["ownRun"], true);
        record(
            json!({"phase":"finish","requestBytes":serde_json::to_vec(&completion.request).unwrap().len(),"replyBytes":completion.reply.as_ref().map(|value|serde_json::to_vec(value).unwrap().len()),"success":completion.success}),
        );
        Ok(())
    }
}
struct Plugin(GatewayHooks);
#[async_trait]
impl ProviderPlugin for Plugin {
    fn gateway_hooks(&self) -> Option<&GatewayHooks> {
        Some(&self.0)
    }
}
#[tokio::main]
async fn main() {
    nora_provider_plugin_sdk::serve(Plugin(GatewayHooks::new(Hook)))
        .await
        .unwrap();
}
