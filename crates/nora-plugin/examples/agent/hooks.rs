//! Agent hooks as kernel events. The dispatch mode is chosen by the emitter:
//! the agent calls `before_tool_call` with `serial` (the first block wins)
//! and `transform_prompt` with `waterfall` (each listener wraps the rest).

use nora_plugin::Event;

/// Before a tool runs. A listener that returns `Some(reason)` blocks it.
pub struct BeforeToolCall {
    pub tool: String,
    pub input: String,
}

impl Event for BeforeToolCall {
    const NAME: &'static str = "agent/before-tool-call";
    type Value = String;
}

/// The prompt on its way to the model; each listener may rewrite it.
pub struct TransformPrompt(pub String);

impl Event for TransformPrompt {
    const NAME: &'static str = "agent/transform-prompt";
    type Value = String;
}

/// After a tool ran: a notification nobody can veto.
pub struct ToolCalled {
    pub tool: String,
    pub output: String,
}

impl Event for ToolCalled {
    const NAME: &'static str = "agent/tool-called";
    type Value = ();
}
