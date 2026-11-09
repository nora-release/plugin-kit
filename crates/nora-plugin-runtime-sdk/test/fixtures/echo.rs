use nora_plugin_runtime_sdk::protocol::{
    CallToolParams, InitializeParams, InitializeResult, RpcError, ServiceCallParams,
    ToolDefinition, ToolResult, PROTOCOL_VERSION,
};
use nora_plugin_runtime_sdk::{Plugin, PluginContext};
use serde_json::json;

struct Echo;

impl Plugin for Echo {
    async fn initialize(
        &self,
        params: InitializeParams,
        context: PluginContext,
    ) -> Result<InitializeResult, RpcError> {
        if params
            .config
            .get("initializeService")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
        {
            context
                .call_service("fixture.echo", "echo", json!({"during":"initialize"}))
                .await?;
        }
        Ok(InitializeResult {
            protocol_version: PROTOCOL_VERSION,
            services: vec!["example.echo".into()],
            tools: ["echo", "service", "wait"]
                .into_iter()
                .map(|name| ToolDefinition {
                    name: name.into(),
                    description: format!("Fixture {name}"),
                    input_schema: json!({"type":"object"}),
                })
                .collect(),
            ..InitializeResult::default()
        })
    }

    async fn call_tool(
        &self,
        params: CallToolParams,
        context: PluginContext,
    ) -> Result<ToolResult, RpcError> {
        match params.name.as_str() {
            "echo" => Ok(ToolResult::text(params.arguments.to_string())),
            "service" => Ok(ToolResult::text(
                context
                    .call_service("fixture.echo", "echo", params.arguments)
                    .await?
                    .to_string(),
            )),
            "wait" => std::future::pending().await,
            _ => Err(RpcError::invalid_params("Unknown fixture tool")),
        }
    }

    async fn call_service(
        &self,
        params: ServiceCallParams,
        context: PluginContext,
    ) -> Result<serde_json::Value, RpcError> {
        match params.method.as_str() {
            // protocol 2, as a provider's fetch: reads the whole input,
            // answers with the head, then writes the body to its output
            "copy" => {
                let (Some(mut input), Some(mut output)) =
                    (context.take_input_stream(), context.take_output_stream())
                else {
                    return Err(RpcError::invalid_params("copy needs both streams"));
                };
                let body = input.read_to_end(64 * 1024 * 1024).await?;
                tokio::spawn(async move {
                    if output.write(&body).await.is_ok() {
                        let _ = output.finish().await;
                    }
                });
                return Ok(json!({"head": true}));
            }
            // writes `bytes` bytes, then ends, or gives up with `fail`
            "spill" => {
                let mut output = context
                    .take_output_stream()
                    .ok_or_else(|| RpcError::invalid_params("spill needs an output stream"))?;
                let count = params.arguments["bytes"].as_u64().unwrap_or_default();
                let bytes = vec![b'x'; usize::try_from(count).unwrap_or_default()];
                output.write(&bytes).await?;
                if params.arguments["fail"].as_bool() == Some(true) {
                    output.abort(RpcError::new(-32050, "spill failed")).await?;
                } else {
                    output.finish().await?;
                }
                return Ok(json!({"wrote": count}));
            }
            _ => {}
        }
        if params.method == "reverse" {
            return context
                .call_service("fixture.echo", "echo", params.arguments)
                .await;
        }
        if params.method != "echo" {
            return Err(RpcError::method_not_found());
        }
        Ok(params.arguments)
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    nora_plugin_runtime_sdk::serve(Echo).await
}
