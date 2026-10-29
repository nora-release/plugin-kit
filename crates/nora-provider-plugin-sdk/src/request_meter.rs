//! Private per-request permission to send HTTP; no provider secrets cross this channel.
use crate::{PluginError, PluginResult};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    future::Future,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use tokio::{
    io::AsyncWrite,
    sync::{oneshot, Mutex as AsyncMutex},
};

tokio::task_local! {static CURRENT: Arc<dyn MeterSink>;}

/// Wait immediately before sending each HTTP request. Outside a metered fetch,
/// or with a host that did not negotiate metering, this is a no-op.
pub async fn before_http_request() -> PluginResult<()> {
    let sink = CURRENT.try_with(Arc::clone).ok();
    match sink {
        Some(sink) => sink.wait(false).await,
        None => Ok(()),
    }
}

/// A background task may inherit its originating request's meter. An active
/// fetch always takes precedence, including when a long-lived client is reused.
#[derive(Clone, Default)]
pub struct RequestMeterContext(Option<Arc<dyn MeterSink>>);
impl RequestMeterContext {
    pub fn capture() -> Self {
        Self(CURRENT.try_with(Arc::clone).ok())
    }
    pub async fn scope<F: Future>(&self, future: F) -> F::Output {
        if CURRENT.try_with(|_| ()).is_ok() {
            return future.await;
        }
        match &self.0 {
            Some(sink) => CURRENT.scope(sink.clone(), future).await,
            None => future.await,
        }
    }
    /// One request for cancellation cleanup, counted immediately without waiting
    /// behind the cancelled request. Nora limits this scope to three seconds.
    pub async fn cleanup_scope<F: Future>(&self, future: F) -> F::Output {
        match &self.0 {
            Some(sink) => {
                CURRENT
                    .scope(
                        Arc::new(Cleanup {
                            inner: sink.clone(),
                            used: AtomicBool::new(false),
                        }) as Arc<dyn MeterSink>,
                        future,
                    )
                    .await
            }
            None => future.await,
        }
    }
}
#[async_trait::async_trait]
trait MeterSink: Send + Sync {
    async fn wait(&self, cleanup: bool) -> PluginResult<()>;
}
struct Cleanup {
    inner: Arc<dyn MeterSink>,
    used: AtomicBool,
}
#[async_trait::async_trait]
impl MeterSink for Cleanup {
    async fn wait(&self, _: bool) -> PluginResult<()> {
        if self.used.swap(true, Ordering::AcqRel) {
            return Err(failed());
        }
        self.inner.wait(true).await
    }
}
#[derive(Default)]
pub(crate) struct Acknowledgments {
    next: AtomicU64,
    pending: Mutex<BTreeMap<(u64, u64), oneshot::Sender<bool>>>,
}
impl Acknowledgments {
    pub fn receive(&self, params: &Value) {
        let (Some(id), Some(meter), Some(success)) = (
            params.get("id").and_then(Value::as_u64),
            params.get("meterID").and_then(Value::as_u64),
            params.get("success").and_then(Value::as_bool),
        ) else {
            return;
        };
        if let Some(sender) = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&(id, meter))
        {
            let _ = sender.send(success);
        }
    }
}
struct Pending<W: AsyncWrite + Unpin + Send + 'static> {
    request: u64,
    meter: u64,
    acks: Arc<Acknowledgments>,
    writer: Arc<AsyncMutex<W>>,
    armed: bool,
}
impl<W: AsyncWrite + Unpin + Send + 'static> Drop for Pending<W> {
    fn drop(&mut self) {
        self.acks
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&(self.request, self.meter));
        if !self.armed {
            return;
        }
        let writer = self.writer.clone();
        let request = self.request;
        let meter = self.meter;
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = crate::runtime::write_frame(
                    &writer,
                    json!({"id":request,"event":"httpRequestCancelled","meterID":meter}),
                )
                .await;
            });
        }
    }
}
struct Sink<W> {
    request: u64,
    writer: Arc<AsyncMutex<W>>,
    acks: Arc<Acknowledgments>,
    enabled: Arc<AtomicBool>,
}
#[async_trait::async_trait]
impl<W: AsyncWrite + Unpin + Send + 'static> MeterSink for Sink<W> {
    async fn wait(&self, cleanup: bool) -> PluginResult<()> {
        if !self.enabled.load(Ordering::Acquire) {
            return Ok(());
        }
        // `try_update` is the later name, and it is still unstable on the 1.88 MSRV.
        #[allow(deprecated)]
        let meter = self
            .acks
            .next
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| failed())?
            + 1;
        let (send, receive) = oneshot::channel();
        {
            let mut pending = self.acks.pending.lock().unwrap_or_else(|e| e.into_inner());
            if pending.len() >= 1024 {
                return Err(PluginError::new(
                    "busy",
                    "Too many HTTP requests are waiting to be sent.",
                ));
            }
            pending.insert((self.request, meter), send);
        }
        let mut pending = Pending {
            request: self.request,
            meter,
            acks: self.acks.clone(),
            writer: self.writer.clone(),
            armed: true,
        };
        crate::runtime::write_frame(&self.writer,json!({"id":self.request,"event":if cleanup{"httpRequestCleanup"}else{"httpRequest"},"meterID":meter})).await.map_err(|_|failed())?;
        // The host owns the waiting bound. Cancelling the SDK request drops this
        // receiver immediately; no timer may let an unapproved request escape.
        let result = receive.await;
        pending.armed = false;
        match result {
            Ok(true) => Ok(()),
            _ => Err(failed()),
        }
    }
}
fn failed() -> PluginError {
    PluginError::new(
        "http_request_refused",
        "The provider request was cancelled or could not be sent within its request limits.",
    )
}
pub(crate) async fn scope<W, F: Future>(
    request: u64,
    writer: Arc<AsyncMutex<W>>,
    acks: Arc<Acknowledgments>,
    enabled: Arc<AtomicBool>,
    future: F,
) -> F::Output
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    let sink: Arc<dyn MeterSink> = Arc::new(Sink {
        request,
        writer,
        acks,
        enabled,
    });
    CURRENT.scope(sink, future).await
}

#[cfg(test)]
#[path = "../test/request_meter.test.rs"]
mod tests;
