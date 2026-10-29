//! Echo Example: a provider plugin that answers every chat with the user's
//! last message, so it works offline and needs no account.
//!
//! Nora starts this executable and speaks JSONL on stdin and stdout. The SDK's
//! `serve` handles that protocol; the plugin only implements the calls below.

use async_trait::async_trait;
use futures::stream;
use nora_provider_plugin_sdk::{
    serve, CallContext, FetchRequest, FetchResponse, InitializeParams, PluginError, PluginResult,
    ProviderPlugin, API_VERSION,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::OnceLock;

struct Echo {
    models: OnceLock<Value>,
}

#[async_trait]
impl ProviderPlugin for Echo {
    /// Nora sends the package manifest once; the models come from its
    /// provider.
    async fn initialize(&self, params: InitializeParams) -> PluginResult<Value> {
        let _ = self.models.set(Value::Array(params.provider()?.models));
        Ok(json!({ "apiVersion": API_VERSION }))
    }

    async fn models(&self, _context: CallContext) -> PluginResult<Value> {
        Ok(self.models.get().cloned().unwrap_or_else(|| json!([])))
    }

    /// No sign-in: the provider is ready as soon as it is installed.
    async fn auth_methods(&self, _context: CallContext) -> PluginResult<Value> {
        Ok(json!([]))
    }

    /// Every model request arrives here as the HTTP request Nora would have
    /// sent to `baseURL`. A real provider forwards it upstream; Echo answers
    /// it in the OpenAI chat completions format itself.
    async fn fetch(&self, request: FetchRequest) -> PluginResult<FetchResponse> {
        if !request.url.ends_with("/chat/completions") {
            return Ok(reply(
                404,
                "application/json",
                json!({ "error": { "message": "Echo only answers chat completions." } })
                    .to_string(),
            ));
        }
        let body: Value = request
            .decode_body()?
            .and_then(|body| serde_json::from_slice(&body).ok())
            .ok_or_else(|| PluginError::invalid("The chat request body is not JSON."))?;
        let model = body["model"].as_str().unwrap_or("echo").to_owned();
        let answer = format!("You said: {}", last_user_text(&body));
        if body["stream"].as_bool() == Some(true) {
            return Ok(stream_reply(&model, &answer));
        }
        Ok(reply(
            200,
            "application/json",
            json!({
                "id": "echo", "object": "chat.completion", "model": model,
                "choices": [{ "index": 0, "message": { "role": "assistant", "content": answer }, "finish_reason": "stop" }],
                "usage": usage(&answer),
            })
            .to_string(),
        ))
    }
}

/// The text of the last user message, whether sent as a string or as parts.
fn last_user_text(body: &Value) -> String {
    let message = body["messages"].as_array().and_then(|messages| {
        messages
            .iter()
            .rev()
            .find(|message| message["role"] == "user")
    });
    match message.map(|message| &message["content"]) {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

fn usage(answer: &str) -> Value {
    let words = answer.split_whitespace().count();
    json!({ "prompt_tokens": 0, "completion_tokens": words, "total_tokens": words })
}

fn reply(status: u16, content_type: &str, body: String) -> FetchResponse {
    FetchResponse {
        status,
        headers: BTreeMap::from([("content-type".into(), content_type.into())]),
        body: Box::pin(stream::iter([Ok(body.into_bytes())])),
    }
}

/// The answer as server-sent events, one word at a time.
fn stream_reply(model: &str, answer: &str) -> FetchResponse {
    let chunk = |delta: Value, finish: Value, usage: Option<Value>| {
        let mut event = json!({
            "id": "echo", "object": "chat.completion.chunk", "model": model,
            "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }],
        });
        if let Some(usage) = usage {
            event["usage"] = usage;
        }
        Ok(format!("data: {event}\n\n").into_bytes())
    };
    let mut events = vec![chunk(json!({ "role": "assistant" }), Value::Null, None)];
    for (index, word) in answer.split(' ').enumerate() {
        let text = if index == 0 {
            word.to_owned()
        } else {
            format!(" {word}")
        };
        events.push(chunk(json!({ "content": text }), Value::Null, None));
    }
    events.push(chunk(json!({}), json!("stop"), Some(usage(answer))));
    events.push(Ok(b"data: [DONE]\n\n".to_vec()));
    FetchResponse {
        status: 200,
        headers: BTreeMap::from([("content-type".into(), "text/event-stream".into())]),
        body: Box::pin(stream::iter(events)),
    }
}

#[tokio::main]
async fn main() {
    if let Err(error) = serve(Echo {
        models: OnceLock::new(),
    })
    .await
    {
        eprintln!("Echo Example stopped: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_user_message_is_echoed_from_text_or_parts() {
        let body = json!({ "messages": [
            { "role": "user", "content": "first" },
            { "role": "assistant", "content": "ignored" },
            { "role": "user", "content": [{ "type": "text", "text": "hello" }, { "type": "text", "text": "Nora" }] },
        ]});
        assert_eq!(last_user_text(&body), "hello Nora");
        assert_eq!(last_user_text(&json!({ "messages": [] })), "");
    }
}
