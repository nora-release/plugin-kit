use super::*;
use crate::{serve_io, InitializeParams, PluginResult, ProviderPlugin};
use serde_json::json;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

struct Selector(bool);
#[async_trait::async_trait]
impl ProviderPlugin for Selector {
    fn model_resolution(&self) -> bool {
        self.0
    }
    async fn initialize(&self, _: InitializeParams) -> PluginResult<Value> {
        Ok(json!({"apiVersion":1,"capabilities":{"sdkFeatures":{"modelResolution":1}}}))
    }
    async fn resolve_model(
        &self,
        request: ResolveModelRequest,
    ) -> PluginResult<Option<ModelResolution>> {
        assert_eq!(request.context.account, "first");
        assert_eq!(
            request.context.credentials.unwrap()["access"],
            "private-account-token"
        );
        assert_eq!(request.session_id.as_deref(), Some("session-one"));
        if request.force {
            let previous = request.previous_failure.unwrap();
            assert_eq!(previous.selection_id.as_deref(), Some("selection-one"));
            assert_eq!(previous.status, 400);
            return Ok(None);
        }
        Ok(Some(ModelResolution {
            model: request.model,
            headers: BTreeMap::from([("session-token".into(), "private-selection-token".into())]),
            selection_id: Some("selection-one".into()),
        }))
    }
}

async fn exchange(enabled: bool) -> (Value, Value, Value) {
    let (client, server) = tokio::io::duplex(128 * 1024);
    let (input, output) = tokio::io::split(server);
    let task = tokio::spawn(serve_io(Arc::new(Selector(enabled)), input, output));
    let (output, mut input) = tokio::io::split(client);
    let mut output = BufReader::new(output);
    let mut replies = Vec::new();
    for (id, method, params) in [
        (
            1,
            "initialize",
            json!({"apiVersion":2,"pluginID":"fixture","pluginDirectory":"/plugin","dataDirectory":"/accounts","manifest":{}}),
        ),
        (
            2,
            "resolveModel",
            json!({"account":"first","credentials":{"type":"api","access":"private-account-token"},"model":{"id":"auto"},"prompt":"hello","sessionID":"session-one"}),
        ),
        (
            3,
            "resolveModel",
            json!({"account":"first","credentials":{"type":"api","access":"private-account-token"},"model":{"id":"auto"},"prompt":"hello","sessionID":"session-one","force":true,"previousFailure":{"status":400,"body":"not served","selectionID":"selection-one"}}),
        ),
    ] {
        let message = serde_json::to_string(&json!({"id":id,"method":method,"params":params}))
            .unwrap()
            + "\n";
        input.write_all(message.as_bytes()).await.unwrap();
        let mut line = String::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            output.read_line(&mut line),
        )
        .await
        .unwrap()
        .unwrap();
        replies.push(serde_json::from_str::<Value>(&line).unwrap());
    }
    task.abort();
    (replies.remove(0), replies.remove(0), replies.remove(0))
}

#[tokio::test]
async fn typed_resolution_is_optional_and_keeps_private_values_out_of_debug() {
    let (initialize, resolution, retry) = exchange(true).await;
    assert_eq!(
        initialize.pointer("/result/capabilities/sdkFeatures/modelResolution"),
        Some(&json!(1))
    );
    assert_eq!(
        resolution.pointer("/result/selectionID"),
        Some(&json!("selection-one"))
    );
    assert!(retry["result"].is_null());
    let resolution: ModelResolution = serde_json::from_value(resolution["result"].clone()).unwrap();
    assert!(!format!("{resolution:?}").contains("private"));
}

#[tokio::test]
async fn initialize_cannot_claim_model_resolution_without_the_implementation() {
    let (initialize, resolution, _) = exchange(false).await;
    assert!(initialize
        .pointer("/result/capabilities/sdkFeatures/modelResolution")
        .is_none());
    assert_eq!(
        resolution.pointer("/error/code"),
        Some(&json!("method_not_supported"))
    );
}
