//! Protocol 2 streams: flow-controlled bytes beside a call. One table per
//! connection, the same for the host and a plugin; only the parity of the
//! stream ids each side allocates differs.

use crate::protocol::{
    stream_frame_cost, Request, RpcError, StreamCreditParams, StreamDataParams, MAX_STREAM_CHUNK,
    MAX_STREAM_WINDOW, MIN_STREAM_WINDOW, STREAM_CREDIT, STREAM_DATA, STREAM_FRAME_COST,
};
use base64::Engine;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, Notify};

/// Where a table's frames go: the connection's writer.
pub trait FrameSink: Send + Sync + 'static {
    /// Queues a frame; false once the connection is gone.
    fn send(&self, frame: Value) -> Pin<Box<dyn Future<Output = bool> + Send + '_>>;
    /// Queues a frame without waiting, from a `Drop`.
    fn try_send(&self, frame: Value) -> bool;
}

impl FrameSink for mpsc::Sender<Value> {
    fn send(&self, frame: Value) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        Box::pin(async move { mpsc::Sender::send(self, frame).await.is_ok() })
    }

    fn try_send(&self, frame: Value) -> bool {
        mpsc::Sender::try_send(self, frame).is_ok()
    }
}

/// Which end of the connection a table serves: the host allocates odd
/// stream ids, a plugin even ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Host,
    Plugin,
}

impl Side {
    fn first_id(self) -> u64 {
        match self {
            Self::Host => 1,
            Self::Plugin => 2,
        }
    }

    fn owns(self, id: u64) -> bool {
        id != 0 && (id % 2 == 1) == (self == Self::Host)
    }
}

/// A call whose streams these are: one this side sent (`local`) or one the
/// other side sent. Request ids of the two sides are separate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CallKey {
    pub local: bool,
    pub id: u64,
}

/// A frame the other side should not have sent; the connection fails.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamViolation(pub String);

impl std::fmt::Display for StreamViolation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "invalid stream frame: {}", self.0)
    }
}

impl std::error::Error for StreamViolation {}

pub struct StreamTable {
    side: Side,
    read_window: u64,
    sink: Arc<dyn FrameSink>,
    state: Mutex<State>,
}

struct State {
    write_window: u64,
    next_local: u64,
    max_remote: u64,
    readers: BTreeMap<u64, ReaderSlot>,
    writers: BTreeMap<u64, WriterSlot>,
    calls: BTreeMap<CallKey, BTreeSet<u64>>,
    closed: Option<RpcError>,
}

struct ReaderSlot {
    call: CallKey,
    chunks: mpsc::UnboundedSender<Result<Vec<u8>, RpcError>>,
    /// Credit granted and not yet spent: what the writer may still send.
    granted: u64,
}

struct WriterSlot {
    call: CallKey,
    credit: u64,
    wake: Arc<Notify>,
    closed: Option<RpcError>,
}

impl StreamTable {
    /// `read_window` is the credit this side gives each stream it reads.
    pub fn new(side: Side, read_window: u64, sink: Arc<dyn FrameSink>) -> Arc<Self> {
        Arc::new(Self {
            side,
            read_window: read_window.clamp(MIN_STREAM_WINDOW, MAX_STREAM_WINDOW),
            sink,
            state: Mutex::new(State {
                write_window: crate::protocol::DEFAULT_STREAM_WINDOW,
                next_local: side.first_id(),
                max_remote: 0,
                readers: BTreeMap::new(),
                writers: BTreeMap::new(),
                calls: BTreeMap::new(),
                closed: None,
            }),
        })
    }

    pub fn read_window(&self) -> u64 {
        self.read_window
    }

    /// Takes the other side's `streamWindow` from `initialize`.
    pub fn set_write_window(&self, window: Option<u64>) -> Result<(), StreamViolation> {
        let window = window.unwrap_or(crate::protocol::DEFAULT_STREAM_WINDOW);
        if !(MIN_STREAM_WINDOW..=MAX_STREAM_WINDOW).contains(&window) {
            return Err(StreamViolation("streamWindow is out of range".into()));
        }
        self.lock().write_window = window;
        Ok(())
    }

    /// A stream this side reads, for a call it sends.
    pub fn open_reader(self: &Arc<Self>, call: CallKey) -> Result<StreamReader, RpcError> {
        let mut state = self.lock();
        let id = state.allocate(self.side)?;
        Ok(self.register_reader(&mut state, call, id))
    }

    /// A stream this side writes, for a call it sends.
    pub fn open_writer(self: &Arc<Self>, call: CallKey) -> Result<StreamWriter, RpcError> {
        let mut state = self.lock();
        let id = state.allocate(self.side)?;
        Ok(self.register_writer(&mut state, call, id))
    }

    /// The streams a call from the other side names: `input` this side
    /// reads, `output` it writes. Ids must be the other side's and new.
    pub fn accept(
        self: &Arc<Self>,
        call: CallKey,
        input: Option<u64>,
        output: Option<u64>,
    ) -> Result<(Option<StreamReader>, Option<StreamWriter>), RpcError> {
        let mut state = self.lock();
        if let Some(error) = &state.closed {
            return Err(error.clone());
        }
        let mut ids: Vec<u64> = input.into_iter().chain(output).collect();
        ids.sort_unstable();
        ids.dedup();
        if ids.len() != usize::from(input.is_some()) + usize::from(output.is_some())
            || ids
                .iter()
                .any(|id| self.side.owns(*id) || *id <= state.max_remote)
        {
            return Err(RpcError::invalid_params("Invalid or reused stream id"));
        }
        if let Some(last) = ids.last() {
            state.max_remote = *last;
        }
        let reader = input.map(|id| self.register_reader(&mut state, call, id));
        let writer = output.map(|id| self.register_writer(&mut state, call, id));
        Ok((reader, writer))
    }

    fn register_reader(
        self: &Arc<Self>,
        state: &mut State,
        call: CallKey,
        id: u64,
    ) -> StreamReader {
        let (chunks, received) = mpsc::unbounded_channel();
        state.readers.insert(
            id,
            ReaderSlot {
                call,
                chunks,
                granted: self.read_window,
            },
        );
        state.calls.entry(call).or_default().insert(id);
        StreamReader {
            table: Arc::clone(self),
            id,
            received,
            spent: 0,
            done: false,
            lease: None,
        }
    }

    fn register_writer(
        self: &Arc<Self>,
        state: &mut State,
        call: CallKey,
        id: u64,
    ) -> StreamWriter {
        state.writers.insert(
            id,
            WriterSlot {
                call,
                credit: state.write_window,
                wake: Arc::new(Notify::new()),
                closed: None,
            },
        );
        state.calls.entry(call).or_default().insert(id);
        StreamWriter {
            table: Arc::clone(self),
            id,
            done: false,
            lease: None,
        }
    }

    /// Handles a `streams/data` from the other side.
    pub fn on_data(&self, value: Value) -> Result<(), StreamViolation> {
        let data: StreamDataParams = serde_json::from_value(value)
            .map_err(|_| StreamViolation("malformed streams/data".into()))?;
        if data.error.is_some() && !data.end {
            return Err(StreamViolation("an error without end".into()));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data.data.as_bytes())
            .map_err(|_| StreamViolation("data is not base64".into()))?;
        let mut state = self.lock();
        if state.closed.is_some() {
            return Ok(());
        }
        if state.writers.contains_key(&data.stream) {
            return Err(StreamViolation("data from the stream's reader".into()));
        }
        let Some(slot) = state.readers.get_mut(&data.stream) else {
            return if state.known(self.side, data.stream) {
                Ok(())
            } else {
                Err(StreamViolation("data for an unknown stream".into()))
            };
        };
        // the closing frame is free: there is one per stream
        let cost = if data.end && bytes.is_empty() {
            0
        } else {
            stream_frame_cost(bytes.len())
        };
        if cost > slot.granted {
            return Err(StreamViolation("data beyond the reader's credit".into()));
        }
        slot.granted -= cost;
        if !bytes.is_empty() {
            let _ = slot.chunks.send(Ok(bytes));
        }
        if data.end {
            if let Some(error) = data.error {
                let _ = slot.chunks.send(Err(error));
            }
            state.forget(data.stream);
        }
        Ok(())
    }

    /// Handles a `streams/credit` from the other side.
    pub fn on_credit(&self, value: Value) -> Result<(), StreamViolation> {
        let credit: StreamCreditParams = serde_json::from_value(value)
            .map_err(|_| StreamViolation("malformed streams/credit".into()))?;
        let mut state = self.lock();
        if state.closed.is_some() {
            return Ok(());
        }
        if state.readers.contains_key(&credit.stream) {
            return Err(StreamViolation("credit from the stream's writer".into()));
        }
        let Some(slot) = state.writers.get_mut(&credit.stream) else {
            return if state.known(self.side, credit.stream) {
                Ok(())
            } else {
                Err(StreamViolation("credit for an unknown stream".into()))
            };
        };
        slot.credit = slot.credit.saturating_add(credit.bytes);
        slot.wake.notify_waiters();
        Ok(())
    }

    /// Ends every stream of `call`: its readers read `error`, its writers
    /// stop. Frames of these streams still on the wire are ignored.
    pub fn close_call(&self, call: CallKey, error: &RpcError) {
        let mut state = self.lock();
        let ids = state.calls.remove(&call).unwrap_or_default();
        for id in ids {
            state.close_stream(id, error);
        }
    }

    /// Stops this side's writers of `call`, which the other side answered.
    pub fn close_writers(&self, call: CallKey, error: &RpcError) {
        let mut state = self.lock();
        let ids: Vec<u64> = state
            .calls
            .get(&call)
            .map(|ids| {
                ids.iter()
                    .copied()
                    .filter(|id| state.writers.contains_key(id))
                    .collect()
            })
            .unwrap_or_default();
        for id in ids {
            state.close_stream(id, error);
        }
    }

    /// Whether `call` has a stream that hasn't ended.
    pub fn has_open_streams(&self, call: CallKey) -> bool {
        self.lock()
            .calls
            .get(&call)
            .is_some_and(|ids| !ids.is_empty())
    }

    /// The connection ended: every stream ends with `error`.
    pub fn close_all(&self, error: RpcError) {
        let mut state = self.lock();
        let ids: Vec<u64> = state
            .readers
            .keys()
            .chain(state.writers.keys())
            .copied()
            .collect();
        for id in ids {
            state.close_stream(id, &error);
        }
        state.calls.clear();
        state.closed = Some(error);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Gives the writer `bytes` more. Awaited, never dropped: a lost credit
    /// would leave the writer waiting for good.
    async fn grant(&self, id: u64, bytes: u64) {
        {
            let mut state = self.lock();
            let Some(slot) = state.readers.get_mut(&id) else {
                return;
            };
            slot.granted = slot.granted.saturating_add(bytes);
        }
        if let Ok(frame) = serde_json::to_value(Request::notification(
            STREAM_CREDIT,
            serde_json::json!({"stream": id, "bytes": bytes}),
        )) {
            self.sink.send(frame).await;
        }
    }

    /// The largest raw chunk the other side's window takes in one frame.
    fn chunk_size(&self) -> usize {
        let window = self.lock().write_window;
        usize::try_from(window.saturating_sub(STREAM_FRAME_COST))
            .unwrap_or(MAX_STREAM_CHUNK)
            .clamp(1, MAX_STREAM_CHUNK)
    }
}

impl State {
    fn allocate(&mut self, side: Side) -> Result<u64, RpcError> {
        if let Some(error) = &self.closed {
            return Err(error.clone());
        }
        let id = self.next_local;
        self.next_local = id
            .checked_add(2)
            .ok_or_else(|| RpcError::new(-32000, "Stream ids are exhausted"))?;
        debug_assert!(side.owns(id));
        Ok(id)
    }

    /// Whether `id` was ever a stream of this connection.
    fn known(&self, side: Side, id: u64) -> bool {
        if side.owns(id) {
            id < self.next_local
        } else {
            id != 0 && id <= self.max_remote
        }
    }

    fn forget(&mut self, id: u64) {
        let call = self
            .readers
            .remove(&id)
            .map(|slot| slot.call)
            .or_else(|| self.writers.remove(&id).map(|slot| slot.call));
        if let Some(call) = call {
            if let Some(ids) = self.calls.get_mut(&call) {
                ids.remove(&id);
                if ids.is_empty() {
                    self.calls.remove(&call);
                }
            }
        }
    }

    fn close_stream(&mut self, id: u64, error: &RpcError) {
        if let Some(slot) = self.readers.get(&id) {
            let _ = slot.chunks.send(Err(error.clone()));
        }
        if let Some(slot) = self.writers.get_mut(&id) {
            // kept until its writer goes, so it reads why it was closed
            slot.closed = Some(error.clone());
            slot.wake.notify_waiters();
            let call = slot.call;
            if let Some(ids) = self.calls.get_mut(&call) {
                ids.remove(&id);
                if ids.is_empty() {
                    self.calls.remove(&call);
                }
            }
            return;
        }
        self.forget(id);
    }
}

/// Reads one stream. Credit goes back as the bytes are taken.
pub struct StreamReader {
    table: Arc<StreamTable>,
    id: u64,
    received: mpsc::UnboundedReceiver<Result<Vec<u8>, RpcError>>,
    spent: u64,
    done: bool,
    lease: Option<Arc<dyn Send + Sync>>,
}

impl StreamReader {
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Keeps `lease` alive as long as this reader.
    pub fn with_lease(mut self, lease: Arc<dyn Send + Sync>) -> Self {
        self.lease = Some(lease);
        self
    }

    /// The next bytes; None once the writer ended the stream.
    pub async fn read(&mut self) -> Option<Result<Vec<u8>, RpcError>> {
        if self.done {
            return None;
        }
        let Some(chunk) = self.received.recv().await else {
            self.done = true;
            return None;
        };
        match &chunk {
            Ok(bytes) => {
                self.spent = self.spent.saturating_add(stream_frame_cost(bytes.len()));
                if self.spent >= self.table.read_window / 2 {
                    self.table
                        .grant(self.id, std::mem::take(&mut self.spent))
                        .await;
                }
            }
            Err(_) => self.done = true,
        }
        Some(chunk)
    }

    /// Every remaining byte, up to `limit`.
    pub async fn read_to_end(&mut self, limit: usize) -> Result<Vec<u8>, RpcError> {
        let mut all = Vec::new();
        while let Some(chunk) = self.read().await {
            let chunk = chunk?;
            if all.len().saturating_add(chunk.len()) > limit {
                return Err(RpcError::new(-32000, "The stream exceeds its limit"));
            }
            all.extend_from_slice(&chunk);
        }
        Ok(all)
    }

    /// Whether the writer ended the stream, or it failed.
    pub fn is_done(&self) -> bool {
        self.done
    }
}

impl Drop for StreamReader {
    fn drop(&mut self) {
        self.table.lock().forget(self.id);
    }
}

/// Writes one stream, waiting for credit.
pub struct StreamWriter {
    table: Arc<StreamTable>,
    id: u64,
    done: bool,
    lease: Option<Arc<dyn Send + Sync>>,
}

impl StreamWriter {
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Keeps `lease` alive as long as this writer.
    pub fn with_lease(mut self, lease: Arc<dyn Send + Sync>) -> Self {
        self.lease = Some(lease);
        self
    }

    /// Sends `bytes`, in as many frames as the reader's credit allows. A
    /// frame takes what the credit has room for rather than waiting for a
    /// whole chunk: the reader returns credit only once half its window is
    /// spent, so waiting for more than is left could wait for good.
    pub async fn write(&mut self, bytes: &[u8]) -> Result<(), RpcError> {
        if self.done {
            return Err(RpcError::new(-32000, "The stream has ended"));
        }
        let chunk = self.table.chunk_size();
        let mut rest = bytes;
        while !rest.is_empty() {
            let count = self.reserve(rest.len().min(chunk)).await?;
            self.send(&rest[..count], false, None).await?;
            rest = &rest[count..];
        }
        Ok(())
    }

    /// Ends the stream.
    pub async fn finish(mut self) -> Result<(), RpcError> {
        self.end(None).await
    }

    /// Ends the stream with an error the reader reads.
    pub async fn abort(mut self, error: RpcError) -> Result<(), RpcError> {
        self.end(Some(error)).await
    }

    async fn end(&mut self, error: Option<RpcError>) -> Result<(), RpcError> {
        if self.done {
            return Ok(());
        }
        self.done = true;
        {
            let state = self.table.lock();
            if let Some(error) = state
                .writers
                .get(&self.id)
                .and_then(|slot| slot.closed.clone())
            {
                return Err(error);
            }
        }
        let sent = self.send(&[], true, error).await;
        self.table.lock().forget(self.id);
        sent
    }

    /// Takes credit for up to `wanted` bytes, at least one, waiting while
    /// there is no room for a frame; returns how many bytes it took.
    async fn reserve(&self, wanted: usize) -> Result<usize, RpcError> {
        loop {
            let wake = {
                let mut state = self.table.lock();
                let Some(slot) = state.writers.get_mut(&self.id) else {
                    return Err(state
                        .closed
                        .clone()
                        .unwrap_or_else(|| RpcError::new(-32000, "The stream was closed")));
                };
                if let Some(error) = &slot.closed {
                    return Err(error.clone());
                }
                if slot.credit > STREAM_FRAME_COST {
                    let room =
                        usize::try_from(slot.credit - STREAM_FRAME_COST).unwrap_or(usize::MAX);
                    let count = wanted.min(room);
                    slot.credit -= stream_frame_cost(count);
                    return Ok(count);
                }
                Arc::clone(&slot.wake)
            };
            let notified = wake.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            // credit or a close may have come between the check and enable
            {
                let state = self.table.lock();
                match state.writers.get(&self.id) {
                    Some(slot) if slot.closed.is_none() && slot.credit <= STREAM_FRAME_COST => {}
                    _ => continue,
                }
            }
            notified.await;
        }
    }

    async fn send(&self, bytes: &[u8], end: bool, error: Option<RpcError>) -> Result<(), RpcError> {
        let frame = serde_json::to_value(Request::notification(
            STREAM_DATA,
            serde_json::to_value(StreamDataParams {
                stream: self.id,
                data: base64::engine::general_purpose::STANDARD.encode(bytes),
                end,
                error,
            })
            .map_err(|error| RpcError::new(-32603, error.to_string()))?,
        ))
        .map_err(|error| RpcError::new(-32603, error.to_string()))?;
        if self.table.sink.send(frame).await {
            Ok(())
        } else {
            Err(RpcError::new(-32000, "The plugin connection is closed"))
        }
    }
}

impl Drop for StreamWriter {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        let open = {
            let mut state = self.table.lock();
            let open = state
                .writers
                .get(&self.id)
                .is_some_and(|slot| slot.closed.is_none());
            state.forget(self.id);
            open
        };
        // a writer dropped mid-stream tells its reader rather than leaving
        // it waiting for an end that never comes
        if open {
            let frame = serde_json::to_value(Request::notification(
                STREAM_DATA,
                serde_json::json!({
                    "stream": self.id,
                    "end": true,
                    "error": {"code": -32000, "message": "The stream writer stopped"},
                }),
            ));
            if let Ok(frame) = frame {
                self.table.sink.try_send(frame);
            }
        }
    }
}

#[cfg(test)]
#[path = "../test/streams.test.rs"]
mod tests;
