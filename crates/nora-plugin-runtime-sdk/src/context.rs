use crate::protocol::{Request, RpcError, ServiceCallParams, CALL_SERVICE, CANCELLED};
use crate::streams::{FrameSink, StreamReader, StreamWriter};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, Semaphore};

type ServiceReplies = BTreeMap<u64, oneshot::Sender<Result<Value, RpcError>>>;

#[derive(Clone)]
pub struct PluginContext {
    pub(crate) outgoing: mpsc::Sender<Outgoing>,
    replies: Arc<Mutex<ServiceReplies>>,
    next_id: Arc<AtomicU64>,
    closed: Arc<AtomicBool>,
    timeout: Duration,
    request_id: Option<u64>,
    request_active: Option<Arc<AtomicBool>>,
    grants: Arc<RwLock<BTreeSet<String>>>,
    service_calls: Arc<Semaphore>,
    streams: Arc<Mutex<CallStreams>>,
}

#[derive(Default)]
struct CallStreams {
    input: Option<StreamReader>,
    output: Option<StreamWriter>,
}

impl PluginContext {
    pub(crate) fn new(
        outgoing: mpsc::Sender<Outgoing>,
        timeout: Duration,
        max_in_flight: usize,
    ) -> Self {
        Self {
            outgoing,
            replies: Arc::default(),
            next_id: Arc::new(AtomicU64::new(1)),
            closed: Arc::default(),
            timeout,
            request_id: None,
            request_active: None,
            grants: Arc::default(),
            service_calls: Arc::new(Semaphore::new(max_in_flight)),
            streams: Arc::default(),
        }
    }

    pub(crate) fn with_request_id(mut self, id: u64) -> Self {
        self.request_id = Some(id);
        self.request_active = Some(Arc::new(AtomicBool::new(true)));
        self.streams = Arc::default();
        self
    }

    pub(crate) fn with_streams(
        self,
        input: Option<StreamReader>,
        output: Option<StreamWriter>,
    ) -> Self {
        *self
            .streams
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = CallStreams { input, output };
        self
    }

    /// Protocol 2: the stream the host writes for this call, once.
    pub fn take_input_stream(&self) -> Option<StreamReader> {
        self.streams
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .input
            .take()
    }

    /// Protocol 2: the stream this call's answer is written to, once. It
    /// may outlive the call's reply: reply first (say an HTTP status and
    /// headers), then keep writing from a task of its own.
    pub fn take_output_stream(&self) -> Option<StreamWriter> {
        self.streams
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .output
            .take()
    }

    pub(crate) fn set_grants(&self, grants: &[String]) {
        *self
            .grants
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = grants.iter().cloned().collect();
    }

    pub(crate) fn request_scope(&self) -> RequestScope {
        RequestScope(self.request_active.clone())
    }

    /// Calls a host service. The host checks its own grant for every call.
    pub async fn call_service(
        &self,
        service: impl Into<String>,
        method: impl Into<String>,
        arguments: Value,
    ) -> Result<Value, RpcError> {
        if self.closed.load(Ordering::Acquire)
            || self.request_id.is_none()
            || self
                .request_active
                .as_ref()
                .is_none_or(|active| !active.load(Ordering::Acquire))
        {
            return Err(RpcError::new(-32000, "The plugin connection is closed"));
        }
        let service = service.into();
        if !self
            .grants
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(&service)
        {
            return Err(RpcError::permission_denied());
        }
        let _permit = Arc::clone(&self.service_calls)
            .try_acquire_owned()
            .map_err(|_| RpcError::new(-32002, "The host service request limit was reached"))?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let params = ServiceCallParams {
            service,
            method: method.into(),
            arguments,
            parent_request_id: self.request_id,
            input_stream: None,
            output_stream: None,
        };
        let value = serde_json::to_value(Request::new(
            id,
            CALL_SERVICE,
            serde_json::to_value(params).map_err(internal_error)?,
        ))
        .map_err(internal_error)?;
        let (send, receive) = oneshot::channel();
        self.replies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id, send);
        let _pending = PendingService {
            context: self.clone(),
            id,
        };
        tokio::time::timeout(self.timeout, async {
            self.outgoing
                .send(Outgoing::Frame(value))
                .await
                .map_err(|_| RpcError::new(-32000, "The plugin connection is closed"))?;
            receive
                .await
                .map_err(|_| RpcError::new(-32000, "The host stopped responding"))?
        })
        .await
        .map_err(|_| RpcError::new(-32001, "The host service call timed out"))?
    }

    pub(crate) fn reply(&self, id: u64, result: Result<Value, RpcError>) {
        if let Some(reply) = self
            .replies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&id)
        {
            let _ = reply.send(result);
        }
    }

    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::Release);
        let replies = std::mem::take(
            &mut *self
                .replies
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for (_, reply) in replies {
            let _ = reply.send(Err(RpcError::new(
                -32000,
                "The plugin connection is closed",
            )));
        }
    }
}

pub(crate) enum Outgoing {
    Frame(Value),
    Close,
}

/// The connection's writer, as the stream table sends to it.
pub(crate) struct OutgoingSink(pub(crate) mpsc::Sender<Outgoing>);

impl FrameSink for OutgoingSink {
    fn send(
        &self,
        frame: Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send + '_>> {
        Box::pin(async move { self.0.send(Outgoing::Frame(frame)).await.is_ok() })
    }

    fn try_send(&self, frame: Value) -> bool {
        self.0.try_send(Outgoing::Frame(frame)).is_ok()
    }
}

pub(crate) struct RequestScope(Option<Arc<AtomicBool>>);

impl Drop for RequestScope {
    fn drop(&mut self) {
        if let Some(active) = &self.0 {
            active.store(false, Ordering::Release);
        }
    }
}

struct PendingService {
    context: PluginContext,
    id: u64,
}

impl Drop for PendingService {
    fn drop(&mut self) {
        let cancelled = self
            .context
            .replies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.id)
            .is_some();
        if cancelled {
            let notification =
                Request::notification(CANCELLED, serde_json::json!({"requestId":self.id}));
            if let Ok(value) = serde_json::to_value(notification) {
                let _ = self.context.outgoing.try_send(Outgoing::Frame(value));
            }
        }
    }
}

fn internal_error(error: serde_json::Error) -> RpcError {
    RpcError::new(-32603, error.to_string())
}
