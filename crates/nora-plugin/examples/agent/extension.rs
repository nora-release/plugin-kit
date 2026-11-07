//! `agent.tools` as an extension point: many contributors, ordered.
//!
//! Kernel services have one provider each. An extension point is therefore a
//! single service (the registry) that many plugins contribute into; each
//! contribution is registered with `ctx.effect`, so the kernel removes it exactly
//! once when the contributing plugin stops, fails, or is replaced.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use nora_plugin::{Ctx, Result};

pub type ToolFn = Arc<dyn Fn(&str) -> String + Send + Sync>;

pub struct Tool {
    pub name: String,
    /// Lower runs first when two tools are listed; ties keep registration order.
    pub priority: i32,
    /// Which plugin contributed it, for replay and diagnostics.
    pub source: String,
    pub run: ToolFn,
}

#[derive(Default)]
pub struct ToolRegistry {
    next: AtomicU64,
    tools: Mutex<BTreeMap<u64, Arc<Tool>>>,
}

impl ToolRegistry {
    /// Adds `tool` for as long as the plugin that owns `ctx` is active.
    pub fn contribute(self: &Arc<Self>, ctx: &Ctx, tool: Tool) -> Result {
        let registry = Arc::clone(self);
        ctx.effect(move || {
            let id = registry.next.fetch_add(1, Ordering::Relaxed);
            registry.lock().insert(id, Arc::new(tool));
            async move {
                registry.lock().remove(&id);
                Ok(())
            }
        })?;
        Ok(())
    }

    /// What one agent turn sees. Changes after this call wait for the next turn.
    pub fn snapshot(&self) -> Vec<Arc<Tool>> {
        let mut tools: Vec<(u64, Arc<Tool>)> = self
            .lock()
            .iter()
            .map(|(id, tool)| (*id, Arc::clone(tool)))
            .collect();
        tools.sort_by_key(|(id, tool)| (tool.priority, *id));
        tools.into_iter().map(|(_, tool)| tool).collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<u64, Arc<Tool>>> {
        self.tools.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
