# plugin-kit

The plugin system Nora is built on. Nora's runtime registers its extension
points here, and its own features — tools, hooks, compaction, memory — are
plugins like any other.

This repository holds what a plugin author needs; Nora holds the host modules
that use it.

| Part | What it is |
| --- | --- |
| [`nora-plugin`](crates/nora-plugin) | The kernel: services, dependency-driven lifecycle, events, leases |
| [`nora-plugin-protocol`](crates/nora-plugin-protocol) | The messages a host and a process plugin exchange ([PROTOCOL.md](PROTOCOL.md)) |
| [`nora-plugin-runtime-sdk`](crates/nora-plugin-runtime-sdk) | Write a process plugin in Rust |
| [`sdk-go`](sdk-go) | Write a process plugin in Go (`github.com/nora-release/plugin-kit/sdk-go`) |
| [`sdk-swift`](sdk-swift) | Write a process plugin in Swift |

## The model

A plugin declares the services it needs. The kernel starts it once they all
exist, restarts it when one is replaced, stops it when one goes away, and
releases everything it registered exactly once, newest first.

```rust
use std::sync::Arc;

use nora_plugin::{Ctx, Plugin, Result};

pub struct ReadFileTool;

impl Plugin for ReadFileTool {
    // Waits for a ToolRegistry; restarts if it is replaced.
    type Deps = Arc<ToolRegistry>;

    async fn apply(&self, ctx: &Ctx, tools: Arc<ToolRegistry>) -> Result {
        // Contributed with ctx.effect: removed when this plugin stops.
        tools.contribute(ctx, read_file_tool())
    }
}
```

What `apply` can do through its `Ctx`:

| Call | Effect, released when the plugin stops |
| --- | --- |
| `ctx.provide(value)` / `ctx.provide_as::<dyn Trait>(arc)` | Provides a singleton service; a second provider is refused |
| `ctx.on::<Event>(listener)` | Listens to an event |
| `ctx.effect(\|\| async { … })` | Runs now, registers the returned cleanup |
| `ctx.plugin(child)` | Mounts a child; it ends with its parent (a scope) |
| `ctx.gate()` | A gate whose passes the stop waits for (work in flight) |

Events are dispatched in the mode the emitter picks:

| Mode | Order | Result |
| --- | --- | --- |
| `ctx.emit(e)` | every listener starts at once | ignored; failures only logged |
| `ctx.parallel(e).await` | concurrent, waits for all | failures in registration order |
| `ctx.serial(e).await` | one at a time | the first `Some` stops the dispatch |
| `ctx.waterfall(e).terminal(t).await` | first registered is outermost | each listener calls `next` |

## Guarantees

Each one is pinned by a test in [`crates/nora-plugin/tests`](crates/nora-plugin/tests).

| | Guarantee |
| --- | --- |
| K1 | A plugin does not start before every dependency exists |
| K2 | Everything a plugin registered is released once, newest first |
| K3 | A failed `apply` rolls back what it registered and affects no other plugin |
| K4 | Replacing a service restarts only the plugins that use it |
| K5 | Ending a scope ends everything mounted inside it |
| K6 | A panic in `apply` fails only that plugin |
| K7 | Loading and unloading repeatedly leaves nothing behind |
| K9 | Stopping waits for work in flight; no new work starts meanwhile |

Also checked: a stopped plugin's context can register nothing, a failed plugin
retries only when a dependency changes, and shutdown unloads newest first.

## Try it

```sh
make example   # an agent's tools and hooks as plugins, step by step
make check     # format, clippy with warnings denied, tests, the SDKs' tests
```

Each SDK ships an echo plugin (`nora-plugin-runtime-sdk-fixture`,
[`sdk-go/examples/echo`](sdk-go/examples/echo), `nora-plugin-echo`) with an
`echo` tool and an `example.echo` service.

The example in [`crates/nora-plugin/examples/agent`](crates/nora-plugin/examples/agent)
builds an `agent.tools` extension point, a `before_tool_call` guard, a prompt
waterfall, a session scope and a service swap, and reads one tool snapshot per
turn.

## Provider plugins

A plugin that adds a model provider runs in Nora's router and answers the
service `ai.provider`: sign in, renew, list models, connect or send a
request, read usage, check in. In Rust, implement
`nora_plugin_runtime_sdk::provider::Provider` for the questions the
manifest says the plugin answers and serve it; the rest are answered as
unsupported:

```rust
use nora_plugin_runtime_sdk::protocol::RpcError;
use nora_plugin_runtime_sdk::provider::{serve_provider, Connection, Model, Provider, Question};

struct Acme;

impl Provider for Acme {
    async fn models(&self, _question: Question) -> Result<Vec<Model>, RpcError> {
        Ok(vec![Model::new("acme-large")])
    }

    async fn connect(&self, question: Question) -> Result<Connection, RpcError> {
        let token: String = question.credentials()?;
        Ok(Connection {
            headers: [("Authorization".into(), format!("Bearer {token}"))].into(),
            ..Connection::default()
        })
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    serve_provider(Acme).await
}
```

The router keeps every account's credentials and hands them back with
each question; `provider::sign_in_needed` and `provider::rate_limited` are
the errors it acts on. The manifest, and what each question means, are in
Nora's `docs/provider-plugins.md`; `provider/fetch` uses protocol 2's
streams.

## Rules for `apply`

All lifecycle transitions run on one driver task, one at a time. `apply` runs
on it, so it may provide services and mount children, but must never wait for
another plugin to become active.

## Not here yet

- The plugin manifest and its install and trust model
- A host for process plugins (Nora's runtime has its own)
- Scoped service names (one provider per workspace or session)
- Starting independent plugins concurrently

## License

MIT
