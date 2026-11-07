//! Nora's plugin kernel.
//!
//! A plugin declares the services it needs (`Plugin::Deps`) and, in `apply`,
//! provides services, listens to events and registers cleanups through its
//! [`Ctx`]. The kernel starts a plugin once its dependencies exist, restarts
//! it when one is replaced, stops it when one goes away, and releases what it
//! registered exactly once, newest first.
//!
//! One driver task owns every lifecycle transition, so they never interleave.
//! `apply` runs on that task: it may mount children and provide services, but
//! must not wait for another plugin to become active.

mod ctx;
mod deps;
mod error;
mod events;
mod gate;

pub use ctx::{Ctx, DependencyDiagnostic, PluginHandle, PluginInfo, Snapshot, State};
pub use deps::{DepKey, Deps, ServiceKey};
pub use error::{Error, NamedServiceAlreadyProvided, Result};
pub use events::{Call, Event, Listener, Mode, Waterfall};
pub use gate::{Gate, Pass};

use std::future::Future;

/// A unit the kernel starts and stops.
pub trait Plugin: Send + Sync + 'static {
    /// The services `apply` receives: `()`, `Arc<T>`, or a tuple of them.
    type Deps: Deps;

    fn apply(&self, ctx: &Ctx, deps: Self::Deps) -> impl Future<Output = Result> + Send;

    /// Required services selected by this instance. Static `Deps` are always
    /// required as well, even when an override returns only named services.
    /// Captured once at mount; replace the plugin to change its declaration.
    fn dependency_keys(&self) -> Vec<ServiceKey> {
        Self::Deps::keys()
            .into_iter()
            .map(ServiceKey::from)
            .collect()
    }

    fn name(&self) -> &str {
        short_type_name::<Self>()
    }
}

fn short_type_name<T: ?Sized>() -> &'static str {
    let full = std::any::type_name::<T>();
    let base = full.split('<').next().unwrap_or(full);
    base.rsplit("::").next().unwrap_or(base)
}
