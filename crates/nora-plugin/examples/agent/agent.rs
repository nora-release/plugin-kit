//! A stand-in for the agent loop, wired the way the composition layer would
//! wire `CodingAgentConfig`: one snapshot of the tools per turn, the hooks
//! dispatched through the kernel.

use std::sync::Arc;

use nora_plugin::Ctx;

use crate::extension::ToolRegistry;
use crate::hooks::{BeforeToolCall, ToolCalled, TransformPrompt};

pub struct TurnReport {
    pub prompt: String,
    pub tools: Vec<String>,
    pub results: Vec<String>,
}

/// One turn: snapshot, transform the prompt, then run each planned call.
pub async fn turn(
    ctx: &Ctx,
    registry: &Arc<ToolRegistry>,
    prompt: &str,
    calls: &[(&str, &str)],
) -> nora_plugin::Result<TurnReport> {
    let tools = registry.snapshot();
    let prompt = ctx
        .waterfall(TransformPrompt(prompt.to_string()))
        .terminal(|event| async move { Ok(Some(event.0.clone())) })
        .await?
        .unwrap_or_default();

    let mut results = Vec::new();
    for (name, input) in calls {
        let Some(tool) = tools.iter().find(|tool| tool.name == *name) else {
            results.push(format!("{name}: no such tool this turn"));
            continue;
        };
        let check = BeforeToolCall {
            tool: tool.name.clone(),
            input: input.to_string(),
        };
        if let Some(reason) = ctx.serial(check).await? {
            results.push(format!("{name}: blocked ({reason})"));
            continue;
        }
        let output = (tool.run)(input);
        ctx.parallel(ToolCalled {
            tool: tool.name.clone(),
            output: output.clone(),
        })
        .await?;
        results.push(format!("{name} [{}]: {output}", tool.source));
    }

    Ok(TurnReport {
        prompt,
        tools: tools.iter().map(|tool| tool.name.clone()).collect(),
        results,
    })
}
