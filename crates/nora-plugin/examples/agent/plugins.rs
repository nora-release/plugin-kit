//! Sample plugins. Each one exercises one idea from the design.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use nora_plugin::{Ctx, Plugin, Result};

use crate::extension::{Tool, ToolRegistry};
use crate::hooks::{BeforeToolCall, ToolCalled, TransformPrompt};

/// The host module that owns the `agent.tools` extension point.
pub struct AgentTools;

impl Plugin for AgentTools {
    type Deps = ();

    async fn apply(&self, ctx: &Ctx, (): ()) -> Result {
        ctx.provide(ToolRegistry::default())?;
        Ok(())
    }
}

/// A built-in tool: contributes into the extension point, nothing else.
pub struct ReadFileTool;

impl Plugin for ReadFileTool {
    type Deps = Arc<ToolRegistry>;

    async fn apply(&self, ctx: &Ctx, tools: Arc<ToolRegistry>) -> Result {
        tools.contribute(
            ctx,
            Tool {
                name: "read_file".into(),
                priority: 0,
                source: "builtin.read-file@1".into(),
                run: Arc::new(|path| format!("<contents of {path}>")),
            },
        )
    }
}

/// A tool the guard below has an opinion about.
pub struct BashTool;

impl Plugin for BashTool {
    type Deps = Arc<ToolRegistry>;

    async fn apply(&self, ctx: &Ctx, tools: Arc<ToolRegistry>) -> Result {
        tools.contribute(
            ctx,
            Tool {
                name: "bash".into(),
                priority: 10,
                source: "builtin.bash@1".into(),
                run: Arc::new(|command| format!("$ {command}\n(ran)")),
            },
        )
    }
}

/// A singleton service with two interchangeable implementations.
pub trait Tokenizer: Send + Sync {
    fn count(&self, text: &str) -> usize;
    fn kind(&self) -> &'static str;
}

pub struct WhitespaceTokenizer;

impl Plugin for WhitespaceTokenizer {
    type Deps = ();

    async fn apply(&self, ctx: &Ctx, (): ()) -> Result {
        struct Words;
        impl Tokenizer for Words {
            fn count(&self, text: &str) -> usize {
                text.split_whitespace().count()
            }
            fn kind(&self) -> &'static str {
                "words"
            }
        }
        ctx.provide_as::<dyn Tokenizer>(Arc::new(Words))?;
        Ok(())
    }
}

pub struct CharTokenizer;

impl Plugin for CharTokenizer {
    type Deps = ();

    async fn apply(&self, ctx: &Ctx, (): ()) -> Result {
        struct Chars;
        impl Tokenizer for Chars {
            fn count(&self, text: &str) -> usize {
                text.chars().count()
            }
            fn kind(&self) -> &'static str {
                "chars"
            }
        }
        ctx.provide_as::<dyn Tokenizer>(Arc::new(Chars))?;
        Ok(())
    }
}

/// Depends on the extension point AND a singleton: it starts only when both
/// are there and restarts by itself when the tokenizer is replaced.
pub struct WordCountTool {
    pub applies: Arc<AtomicUsize>,
}

impl Plugin for WordCountTool {
    type Deps = (Arc<ToolRegistry>, Arc<dyn Tokenizer>);

    async fn apply(&self, ctx: &Ctx, (tools, tokenizer): Self::Deps) -> Result {
        self.applies.fetch_add(1, Ordering::SeqCst);
        let kind = tokenizer.kind();
        tools.contribute(
            ctx,
            Tool {
                name: "count".into(),
                priority: 5,
                source: format!("example.count@1 ({kind})"),
                run: Arc::new(move |text| format!("{} {kind}", tokenizer.count(text))),
            },
        )
    }
}

/// A `before_tool_call` hook: blocks destructive shell commands.
pub struct ShellGuard;

impl Plugin for ShellGuard {
    type Deps = ();

    async fn apply(&self, ctx: &Ctx, (): ()) -> Result {
        ctx.on::<BeforeToolCall>(|call, _| async move {
            let blocked = call.tool == "bash" && call.input.contains("rm -rf");
            Ok(blocked.then(|| "ShellGuard: destructive command".to_string()))
        })?;
        Ok(())
    }
}

/// A `transform_context` hook: wraps the rest of the chain.
pub struct WorkspaceNote {
    pub cwd: &'static str,
}

impl Plugin for WorkspaceNote {
    type Deps = ();

    async fn apply(&self, ctx: &Ctx, (): ()) -> Result {
        let cwd = self.cwd;
        ctx.on::<TransformPrompt>(move |_, call| async move {
            let rest = call.next().await?;
            Ok(rest.map(|prompt| format!("{prompt}\n[workspace: {cwd}]")))
        })?;
        Ok(())
    }
}

/// An observer: tallies tool calls and output, never changes them.
pub struct ToolCounter {
    pub calls: Arc<Mutex<BTreeMap<String, usize>>>,
    pub output_bytes: Arc<AtomicUsize>,
}

impl Plugin for ToolCounter {
    type Deps = ();

    async fn apply(&self, ctx: &Ctx, (): ()) -> Result {
        let calls = Arc::clone(&self.calls);
        let bytes = Arc::clone(&self.output_bytes);
        ctx.on::<ToolCalled>(move |called, _| {
            *calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .entry(called.tool.clone())
                .or_default() += 1;
            bytes.fetch_add(called.output.len(), Ordering::SeqCst);
            async move { Ok(None) }
        })?;
        Ok(())
    }
}

/// A session scope: everything mounted inside it ends with the session.
pub struct Session {
    pub id: &'static str,
}

impl Plugin for Session {
    type Deps = ();

    async fn apply(&self, ctx: &Ctx, (): ()) -> Result {
        ctx.plugin(SessionNotesTool { session: self.id });
        Ok(())
    }

    fn name(&self) -> &str {
        self.id
    }
}

/// A session-scoped tool: exists only while its session does.
pub struct SessionNotesTool {
    pub session: &'static str,
}

impl Plugin for SessionNotesTool {
    type Deps = Arc<ToolRegistry>;

    async fn apply(&self, ctx: &Ctx, tools: Arc<ToolRegistry>) -> Result {
        let session = self.session;
        tools.contribute(
            ctx,
            Tool {
                name: format!("notes_{session}"),
                priority: 20,
                source: format!("session.notes@1 ({session})"),
                run: Arc::new(move |note| format!("[{session}] noted: {note}")),
            },
        )
    }
}

/// A plugin whose `apply` fails after it already contributed: the kernel must
/// roll the contribution back.
pub struct BrokenTool;

impl Plugin for BrokenTool {
    type Deps = Arc<ToolRegistry>;

    async fn apply(&self, ctx: &Ctx, tools: Arc<ToolRegistry>) -> Result {
        tools.contribute(
            ctx,
            Tool {
                name: "half_installed".into(),
                priority: 0,
                source: "example.broken@1".into(),
                run: Arc::new(|_| String::new()),
            },
        )?;
        Err(std::io::Error::other("config file missing").into())
    }
}
