# plugin-kit

The plugin system Nora is built on. Nora's runtime registers its extension
points here, and its own features — tools, hooks, compaction, memory — are
plugins like any other.

This repository holds what a plugin author needs; Nora holds the host modules
that use it.

| Crate | What it is |
| --- | --- |
| [`nora-plugin`](crates/nora-plugin) | The kernel: services, dependency-driven lifecycle, events, leases |

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
make check     # format, clippy with warnings denied, tests
```

The example in [`crates/nora-plugin/examples/agent`](crates/nora-plugin/examples/agent)
builds an `agent.tools` extension point, a `before_tool_call` guard, a prompt
waterfall, a session scope and a service swap, and reads one tool snapshot per
turn.

## Rules for `apply`

All lifecycle transitions run on one driver task, one at a time. `apply` runs
on it, so it may provide services and mount children, but must never wait for
another plugin to become active.

## Not here yet

- The plugin manifest and its install and trust model
- Plugins in other processes and languages, and their SDKs
- Scoped service names (one provider per workspace or session)
- Starting independent plugins concurrently

## License

MIT
