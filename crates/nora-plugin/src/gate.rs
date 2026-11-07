//! In-flight leases: a plugin that hands out work (a tool call, a request)
//! takes a `Gate`; each piece of work holds a `Pass`. When the plugin stops,
//! the gate closes (no new passes) and its cleanup waits until every pass is
//! dropped, so a replaced implementation never runs past its own shutdown.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::sync::Notify;

#[derive(Default)]
pub struct Gate {
    closed: AtomicBool,
    in_flight: AtomicUsize,
    drained: Notify,
}

/// Held for as long as one piece of work runs.
pub struct Pass(Arc<Gate>);

impl Gate {
    /// A pass for one piece of work, or `None` once the plugin is stopping.
    pub fn enter(self: &Arc<Self>) -> Option<Pass> {
        self.in_flight.fetch_add(1, Ordering::SeqCst);
        if self.closed.load(Ordering::SeqCst) {
            self.leave();
            return None;
        }
        Some(Pass(Arc::clone(self)))
    }

    pub fn in_flight(&self) -> usize {
        self.in_flight.load(Ordering::SeqCst)
    }

    pub(crate) async fn close_and_drain(&self) {
        self.closed.store(true, Ordering::SeqCst);
        loop {
            let drained = self.drained.notified();
            if self.in_flight.load(Ordering::SeqCst) == 0 {
                return;
            }
            drained.await;
        }
    }

    fn leave(&self) {
        if self.in_flight.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.drained.notify_waiters();
        }
    }
}

impl Drop for Pass {
    fn drop(&mut self) {
        self.0.leave();
    }
}
