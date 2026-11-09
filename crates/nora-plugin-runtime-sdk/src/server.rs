use crate::context::{Outgoing, OutgoingSink};
use crate::protocol::{
    CallToolParams, CancelledParams, Request, Response, RpcError, ServiceCallParams, CALL_SERVICE,
    CALL_TOOL, CANCELLED, DEFAULT_MAX_FRAME_BYTES, DEFAULT_MAX_IN_FLIGHT, DEFAULT_STREAM_WINDOW,
    INITIALIZE, JSONRPC_VERSION, SHUTDOWN, STREAMS_PROTOCOL_VERSION, STREAM_CREDIT, STREAM_DATA,
};
use crate::streams::{CallKey, Side, StreamTable};
use crate::{transport, Plugin, PluginContext};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, BufReader};
use tokio::sync::mpsc;
use tokio::task::{AbortHandle, JoinSet};

#[derive(Clone, Debug)]
pub struct ServerOptions {
    pub max_frame_bytes: usize,
    pub max_in_flight: usize,
    pub io_timeout: Duration,
    pub service_timeout: Duration,
    /// Protocol 2: the credit this plugin gives each stream it reads.
    pub stream_window: u64,
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
            max_in_flight: DEFAULT_MAX_IN_FLIGHT,
            io_timeout: Duration::from_secs(30),
            // The host owns the parent deadline. This fallback must cover
            // every supported host budget rather than impose a shorter one.
            service_timeout: Duration::from_millis(crate::protocol::MAX_CALL_TIMEOUT_MS),
            stream_window: DEFAULT_STREAM_WINDOW,
        }
    }
}

pub async fn serve(plugin: impl Plugin) -> io::Result<()> {
    serve_io(
        plugin,
        tokio::io::stdin(),
        tokio::io::stdout(),
        ServerOptions::default(),
    )
    .await
}

pub async fn serve_io<P, R, W>(
    plugin: P,
    input: R,
    mut output: W,
    options: ServerOptions,
) -> io::Result<()>
where
    P: Plugin,
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    if options.max_frame_bytes == 0
        || options.max_in_flight == 0
        || options.io_timeout.is_zero()
        || options.service_timeout.is_zero()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Plugin limits must be positive",
        ));
    }
    let (outgoing, mut frames) = mpsc::channel::<Outgoing>(options.max_in_flight.saturating_mul(2));
    let context = PluginContext::new(
        outgoing.clone(),
        options.service_timeout,
        options.max_in_flight,
    );
    let writer_options = options.clone();
    let mut writer = tokio::spawn(async move {
        while let Some(frame) = frames.recv().await {
            let frame = match frame {
                Outgoing::Frame(frame) => frame,
                Outgoing::Close => break,
            };
            tokio::time::timeout(
                writer_options.io_timeout,
                transport::write_frame(&mut output, &frame, writer_options.max_frame_bytes),
            )
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Plugin output timed out"))??;
        }
        Ok::<_, io::Error>(())
    });
    let _abort_writer = AbortWriter(writer.abort_handle());
    let streams = StreamTable::new(
        Side::Plugin,
        options.stream_window,
        Arc::new(OutgoingSink(outgoing.clone())),
    );
    let version = Arc::new(AtomicU32::new(0));
    let plugin = Arc::new(plugin);
    let initialized = Arc::new(AtomicU8::new(0));
    let services = Arc::new(RwLock::new(BTreeSet::new()));
    let mut reader = BufReader::new(input);
    let mut partial_frame = Vec::new();
    let mut jobs = JoinSet::new();
    let mut active: BTreeMap<u64, AbortHandle> = BTreeMap::new();
    let result = async { loop {
        tokio::select! {
            frame = transport::read_frame_buffered(&mut reader, &mut partial_frame, options.max_frame_bytes) => {
                let value = match frame {
                    Ok(Some(value)) => value,
                    Ok(None) => break Ok(()),
                    Err(error) => break Err(error),
                };
                if value.get("method").is_none() {
                    let response: Response = match serde_json::from_value::<Response>(value) {
                        Ok(response) if response.is_valid() => response,
                        _ => break Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid host response")),
                    };
                    if let Some(id) = response.id {
                        context.reply(id, match response.error {
                            Some(error) => Err(error),
                            None => Ok(response.result.unwrap_or(Value::Null)),
                        });
                    }
                    continue;
                }
                let request: Request = match serde_json::from_value::<Request>(value) {
                    Ok(request) if request.jsonrpc == JSONRPC_VERSION => request,
                    _ => {
                        send_response(&outgoing, Response::error(None, RpcError::new(-32600, "Invalid request"))).await?;
                        continue;
                    }
                };
                if request.method == STREAM_DATA || request.method == STREAM_CREDIT {
                    if version.load(Ordering::Acquire) != STREAMS_PROTOCOL_VERSION {
                        break Err(io::Error::new(io::ErrorKind::InvalidData, "A stream frame outside protocol 2"));
                    }
                    let handled = if request.method == STREAM_DATA {
                        streams.on_data(request.params)
                    } else {
                        streams.on_credit(request.params)
                    };
                    if let Err(violation) = handled {
                        break Err(io::Error::new(io::ErrorKind::InvalidData, violation.to_string()));
                    }
                    continue;
                }
                if request.method == CANCELLED {
                    if let Ok(cancelled) = serde_json::from_value::<CancelledParams>(request.params) {
                        // a reply already sent may still have its output
                        // stream running
                        streams.close_call(
                            CallKey { local: false, id: cancelled.request_id },
                            &RpcError::new(-32800, "The plugin request was cancelled"),
                        );
                        if let Some(task) = active.remove(&cancelled.request_id) {
                            task.abort();
                            send_response(&outgoing, Response::error(Some(cancelled.request_id), RpcError::new(-32800, "The plugin request was cancelled"))).await?;
                        }
                    }
                    continue;
                }
                if request.method == SHUTDOWN {
                    if let Some(id) = request.id {
                        send_response(&outgoing, Response::success(id, Value::Null)).await?;
                    }
                    break Ok(());
                }
                let Some(id) = request.id else { continue; };
                if active.contains_key(&id) || active.len() >= options.max_in_flight {
                    send_response(&outgoing, Response::error(Some(id), RpcError::new(-32002, "The plugin request limit was reached"))).await?;
                    continue;
                }
                let plugin = Arc::clone(&plugin);
                let context = context.clone().with_request_id(id);
                let initialized = Arc::clone(&initialized);
                let services = Arc::clone(&services);
                let connection = Connection {
                    streams: Arc::clone(&streams),
                    version: Arc::clone(&version),
                };
                let response_writer = outgoing.clone();
                let task = jobs.spawn(async move {
                    let _scope = context.request_scope();
                    let result = dispatch(&*plugin, request, context, &initialized, &services, &connection).await;
                    let response = match result {
                        Ok(value) => Response::success(id, value),
                        Err(error) => Response::error(Some(id), error),
                    };
                    (id, send_response(&response_writer, response).await)
                });
                active.insert(id, task);
            }
            finished = jobs.join_next(), if !jobs.is_empty() => {
                match finished {
                    Some(Ok((id, result))) => {
                        active.remove(&id);
                        if let Err(error) = result { break Err(error); }
                    }
                    Some(Err(error)) if error.is_cancelled() => {}
                    Some(Err(_)) => break Err(io::Error::other("The plugin request task failed")),
                    None => {}
                }
            }
            finished = &mut writer => {
                break finished.map_err(|_| io::Error::other("The plugin writer task failed"))?;
            }
        }
    }}.await;
    streams.close_all(RpcError::new(-32000, "The plugin connection is closed"));
    context.close();
    jobs.abort_all();
    while jobs.join_next().await.is_some() {}
    let _ = tokio::time::timeout(options.io_timeout, outgoing.send(Outgoing::Close)).await;
    drop(context);
    drop(outgoing);
    if !writer.is_finished() {
        let _ = tokio::time::timeout(options.io_timeout, &mut writer).await;
        writer.abort();
    }
    result
}

struct AbortWriter(AbortHandle);

impl Drop for AbortWriter {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// What a call needs of its connection: its streams, and the version the
/// host chose.
struct Connection {
    streams: Arc<StreamTable>,
    version: Arc<AtomicU32>,
}

impl Connection {
    /// The streams a call names, handed to its context.
    fn attach(
        &self,
        id: u64,
        input: Option<u64>,
        output: Option<u64>,
        context: PluginContext,
    ) -> Result<PluginContext, RpcError> {
        if input.is_none() && output.is_none() {
            return Ok(context);
        }
        if self.version.load(Ordering::Acquire) != STREAMS_PROTOCOL_VERSION {
            return Err(RpcError::invalid_params("Streams need protocol 2"));
        }
        let (reader, writer) = self
            .streams
            .accept(CallKey { local: false, id }, input, output)?;
        Ok(context.with_streams(reader, writer))
    }
}

async fn dispatch<P: Plugin>(
    plugin: &P,
    request: Request,
    context: PluginContext,
    initialized: &AtomicU8,
    services: &RwLock<BTreeSet<String>>,
    connection: &Connection,
) -> Result<Value, RpcError> {
    let id = request.id.unwrap_or_default();
    match request.method.as_str() {
        INITIALIZE => {
            let params: crate::protocol::InitializeParams = serde_json::from_value(request.params)
                .map_err(|_| RpcError::invalid_params("Invalid initialization parameters"))?;
            let negotiated = params.protocol_version;
            if !crate::protocol::supported_protocol(negotiated) {
                return Err(RpcError::new(-32004, "Unsupported plugin protocol version"));
            }
            if negotiated == STREAMS_PROTOCOL_VERSION {
                connection
                    .streams
                    .set_write_window(params.stream_window)
                    .map_err(|error| RpcError::invalid_params(error.0))?;
            } else if params.stream_window.is_some() {
                return Err(RpcError::invalid_params("streamWindow needs protocol 2"));
            }
            if initialized
                .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                return Err(RpcError::new(-32600, "The plugin was already initialized"));
            }
            context.set_grants(&params.granted_services);
            let result = plugin
                .initialize(params, context)
                .await
                .and_then(|mut result| {
                    // the version is the host's to choose; the plugin speaks it
                    result.protocol_version = negotiated;
                    result.stream_window = (negotiated == STREAMS_PROTOCOL_VERSION)
                        .then(|| connection.streams.read_window());
                    let names: BTreeSet<_> = result.services.iter().cloned().collect();
                    if names.len() != result.services.len()
                        || names.len() > 1024
                        || names
                            .iter()
                            .any(|name| !crate::protocol::valid_service_name(name))
                    {
                        return Err(RpcError::invalid_params("Invalid provided services"));
                    }
                    *services
                        .write()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = names;
                    Ok(result)
                });
            if result.is_ok() {
                connection.version.store(negotiated, Ordering::Release);
            }
            initialized.store(if result.is_ok() { 2 } else { 0 }, Ordering::Release);
            serde_json::to_value(result?).map_err(|error| RpcError::new(-32603, error.to_string()))
        }
        CALL_TOOL => {
            if initialized.load(Ordering::Acquire) != 2 {
                return Err(RpcError::new(
                    -32600,
                    "Initialize the plugin before calling tools",
                ));
            }
            let params: CallToolParams = serde_json::from_value(request.params)
                .map_err(|_| RpcError::invalid_params("Invalid tool call parameters"))?;
            let context =
                connection.attach(id, params.input_stream, params.output_stream, context)?;
            serde_json::to_value(plugin.call_tool(params, context).await?)
                .map_err(|error| RpcError::new(-32603, error.to_string()))
        }
        CALL_SERVICE => {
            if initialized.load(Ordering::Acquire) != 2 {
                return Err(RpcError::new(
                    -32600,
                    "Initialize the plugin before calling services",
                ));
            }
            let mut params: ServiceCallParams = serde_json::from_value(request.params)
                .map_err(|_| RpcError::invalid_params("Invalid service call parameters"))?;
            if !services
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .contains(&params.service)
            {
                return Err(RpcError::method_not_found());
            }
            // The callback context already carries the actual inbound ID.
            // Wire data cannot change the provenance of reverse calls.
            params.parent_request_id = None;
            let context =
                connection.attach(id, params.input_stream, params.output_stream, context)?;
            plugin.call_service(params, context).await
        }
        _ => Err(RpcError::method_not_found()),
    }
}

async fn send_response(writer: &mpsc::Sender<Outgoing>, response: Response) -> io::Result<()> {
    let value = serde_json::to_value(response)?;
    writer
        .send(Outgoing::Frame(value))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "Plugin output closed"))
}
