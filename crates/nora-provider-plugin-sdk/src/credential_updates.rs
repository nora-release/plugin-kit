use crate::{PluginError, PluginResult, MAX_RESPONSE_FRAME_BYTES};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};
use std::time::Duration;
use tokio::io::AsyncWrite;
use tokio::sync::{oneshot, Mutex as AsyncMutex};

tokio::task_local! { static CURRENT: Arc<dyn CredentialSink>; }

/// Persist renewed account credentials before using a newly issued token.
/// This private event is acknowledged by the host and never becomes HTTP data.
pub async fn save_credentials(credentials: Value) -> PluginResult<()> {
    if !credentials.is_object() {
        return Err(PluginError::invalid(
            "Renewed provider credentials must be an object.",
        ));
    }
    let sink = CURRENT
        .try_with(Arc::clone)
        .map_err(|_| PluginError::unsupported("credential updates outside a provider request"))?;
    sink.save(credentials).await
}

#[async_trait::async_trait]
trait CredentialSink: Send + Sync {
    async fn save(&self, credentials: Value) -> PluginResult<()>;
}

#[derive(Default)]
pub(crate) struct Acknowledgments {
    next: AtomicU64,
    pending: Mutex<BTreeMap<(u64, u64), oneshot::Sender<bool>>>,
}

impl Acknowledgments {
    pub(crate) fn receive(&self, params: &Value) {
        let Some(request) = params.get("id").and_then(Value::as_u64) else {
            return;
        };
        let Some(update) = params.get("updateID").and_then(Value::as_u64) else {
            return;
        };
        let Some(success) = params.get("success").and_then(Value::as_bool) else {
            return;
        };
        if let Some(sender) = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&(request, update))
        {
            let _ = sender.send(success);
        }
    }
}

struct Sink<W> {
    request: u64,
    writer: Arc<AsyncMutex<W>>,
    acknowledgments: Arc<Acknowledgments>,
    enabled: Arc<AtomicBool>,
}

struct Pending {
    request: u64,
    update: u64,
    acknowledgments: Arc<Acknowledgments>,
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.acknowledgments
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&(self.request, self.update));
    }
}

#[async_trait::async_trait]
impl<W: AsyncWrite + Unpin + Send + 'static> CredentialSink for Sink<W> {
    async fn save(&self, credentials: Value) -> PluginResult<()> {
        if !self.enabled.load(Ordering::Acquire) {
            return Err(PluginError::unsupported(
                "credential updates with this Nora host",
            ));
        }
        // `try_update` is the later name, and it is still unstable on the 1.88 MSRV.
        #[allow(deprecated)]
        let update = self
            .acknowledgments
            .next
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| failed())?
            + 1;
        let frame = json!({"id":self.request,"event":"credentials","updateID":update,"credentials":credentials});
        if serde_json::to_vec(&frame).map_err(|_| failed())?.len() > MAX_RESPONSE_FRAME_BYTES {
            return Err(PluginError::invalid(
                "Renewed provider credentials exceed the size limit.",
            ));
        }
        let (sender, receiver) = oneshot::channel();
        {
            let mut pending = self
                .acknowledgments
                .pending
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if pending.len() >= 128 || pending.keys().any(|(request, _)| *request == self.request) {
                return Err(PluginError::new(
                    "busy",
                    "A credential update is already waiting to be saved.",
                ));
            }
            pending.insert((self.request, update), sender);
        }
        let _pending = Pending {
            request: self.request,
            update,
            acknowledgments: self.acknowledgments.clone(),
        };
        crate::runtime::write_frame(&self.writer, frame)
            .await
            .map_err(|_| failed())?;
        match tokio::time::timeout(Duration::from_secs(20), receiver).await {
            Ok(Ok(true)) => Ok(()),
            _ => Err(failed()),
        }
    }
}

fn failed() -> PluginError {
    PluginError::new(
        "credential_update_failed",
        "The provider credentials could not be saved. Retry the request or sign in again.",
    )
}

pub(crate) async fn scope<W, F: Future>(
    request: u64,
    writer: Arc<AsyncMutex<W>>,
    acknowledgments: Arc<Acknowledgments>,
    enabled: Arc<AtomicBool>,
    future: F,
) -> F::Output
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    let sink: Arc<dyn CredentialSink> = Arc::new(Sink {
        request,
        writer,
        acknowledgments,
        enabled,
    });
    CURRENT.scope(sink, future).await
}
