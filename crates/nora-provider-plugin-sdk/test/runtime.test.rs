use super::*;
use crate::{
    serve_io, CallContext, FetchRequest, FetchResponse, InitializeParams, PluginError,
    PluginResult, ProviderPlugin, MAX_HTTP_CHUNK_BYTES,
};
use futures::stream;
use std::sync::atomic::AtomicBool;
use std::task::Poll;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf};
use tokio::time::{timeout, Duration};

struct TestPlugin {
    dropped: Arc<AtomicBool>,
}
#[async_trait::async_trait]
impl ProviderPlugin for TestPlugin {
    async fn initialize(&self, params: InitializeParams) -> PluginResult<Value> {
        assert_eq!(params.plugin_id, "fixture");
        assert_eq!(params.plugin_directory.to_str(), Some("/plugin"));
        Ok(json!({"apiVersion":1}))
    }
    async fn models(&self, context: CallContext) -> PluginResult<Value> {
        Ok(json!([{"id":"fixture-model","account":context.account}]))
    }
    async fn auth_methods(&self, _: CallContext) -> PluginResult<Value> {
        Ok(json!([{"id":"key","type":"api"}]))
    }
    async fn authorize(&self, params: Value) -> PluginResult<Value> {
        Ok(json!({"credentials":{"type":"api","key":params["inputs"]["apiKey"]},"ok":true}))
    }
    async fn callback(&self, params: Value) -> PluginResult<Value> {
        Ok(json!({"session":params["session"],"ok":true}))
    }
    async fn refresh(&self, params: Value) -> PluginResult<Value> {
        Ok(json!({"changed":false,"account":params["account"]}))
    }
    async fn usage(&self, _: CallContext) -> PluginResult<Value> {
        Ok(json!({"available":false}))
    }
    async fn fetch(&self, request: FetchRequest) -> PluginResult<FetchResponse> {
        if request.url == "meter" {
            crate::before_http_request().await?;
            crate::before_http_request().await?;
            return Ok(FetchResponse::bytes(
                200,
                BTreeMap::new(),
                b"metered".to_vec(),
            ));
        }
        if request.url == "renew" {
            crate::save_credentials(
                json!({"access":"synthetic-new-access","refresh":"synthetic-new-refresh"}),
            )
            .await?;
            return Ok(FetchResponse::bytes(200, BTreeMap::new(), b"ok".to_vec()));
        }
        if request.url == "error" {
            return Err(PluginError::authentication("Sign in again."));
        }
        if request.url == "hold" {
            struct Probe(Arc<AtomicBool>);
            impl Drop for Probe {
                fn drop(&mut self) {
                    self.0.store(true, Ordering::Release);
                }
            }
            let probe = Probe(self.dropped.clone());
            return Ok(FetchResponse {
                status: 200,
                headers: BTreeMap::new(),
                body: Box::pin(stream::poll_fn(move |_| {
                    let _ = &probe;
                    Poll::Pending
                })),
            });
        }
        if request.url == "broken" {
            return Ok(FetchResponse {
                status: 200,
                headers: BTreeMap::new(),
                body: Box::pin(stream::iter(vec![
                    Ok(b"partial".to_vec()),
                    Err(PluginError::transport("upstream closed")),
                ])),
            });
        }
        let decoded = request.decode_body()?.unwrap_or_default();
        if request.url == "body-file" {
            return Ok(FetchResponse::bytes(200, BTreeMap::new(), decoded));
        }
        Ok(FetchResponse::bytes(
            429,
            BTreeMap::from([
                ("retry-after".to_string(), "2".to_string()),
                ("echo-length".to_string(), decoded.len().to_string()),
            ]),
            vec![b'x'; 2 * MAX_HTTP_CHUNK_BYTES + 7],
        ))
    }
}

struct Harness {
    input: WriteHalf<DuplexStream>,
    output: BufReader<ReadHalf<DuplexStream>>,
    task: JoinHandle<std::io::Result<()>>,
    dropped: Arc<AtomicBool>,
}
impl Harness {
    fn new() -> Self {
        let (client, server) = tokio::io::duplex(1024 * 1024);
        let (server_input, server_output) = tokio::io::split(server);
        let (client_output, client_input) = tokio::io::split(client);
        let dropped = Arc::new(AtomicBool::new(false));
        let plugin = Arc::new(TestPlugin {
            dropped: dropped.clone(),
        });
        let task = tokio::spawn(serve_io(plugin, server_input, server_output));
        Self {
            input: client_input,
            output: BufReader::new(client_output),
            task,
            dropped,
        }
    }
    async fn send(&mut self, id: u64, method: &str, params: Value) {
        let mut bytes =
            serde_json::to_vec(&json!({"id":id,"method":method,"params":params})).unwrap();
        bytes.push(b'\n');
        self.input.write_all(&bytes).await.unwrap();
        self.input.flush().await.unwrap();
    }
    async fn next(&mut self) -> Value {
        let mut line = String::new();
        timeout(Duration::from_secs(2), self.output.read_line(&mut line))
            .await
            .unwrap()
            .unwrap();
        serde_json::from_str(&line).unwrap()
    }
    async fn initialize(&mut self) {
        self.send(1,"initialize",json!({"apiVersion":2,"pluginID":"fixture","pluginDirectory":"/plugin","directory":"/plugin","dataDirectory":"/accounts","manifest":{}})).await;
        assert_eq!(
            self.next().await,
            json!({"id":1,"result":{"apiVersion":1,"capabilities":{"persistent":true,"bodyFile":true,"httpRequestMeter":true,"accountDiscovery":false,"sdkFeatures":{"subscriptionPolicy":1,"automaticProxy":1}}}})
        );
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[tokio::test]
async fn initialization_and_all_standard_methods_follow_the_native_protocol() {
    let mut harness = Harness::new();
    harness.initialize().await;
    for (id, method, params, expected) in [
        (
            2,
            "models",
            json!({"account":"work"}),
            json!([{"id":"fixture-model","account":"work"}]),
        ),
        (
            3,
            "authMethods",
            json!({}),
            json!([{"id":"key","type":"api"}]),
        ),
        (4, "accounts", json!({}), json!([])),
        (
            5,
            "authorize",
            json!({"inputs":{"apiKey":"test-key"}}),
            json!({"credentials":{"type":"api","key":"test-key"},"ok":true}),
        ),
        (
            6,
            "callback",
            json!({"session":"s"}),
            json!({"session":"s","ok":true}),
        ),
        (
            7,
            "refresh",
            json!({"account":"work","credentials":{"key":"test-key"},"force":true}),
            json!({"changed":false,"account":"work"}),
        ),
        (8, "usage", json!({}), json!({"available":false})),
        (9, "discoverAccounts", json!({}), json!({"accounts":[]})),
    ] {
        harness.send(id, method, params).await;
        assert_eq!(harness.next().await, json!({"id":id,"result":expected}));
    }
}

#[tokio::test]
async fn rejects_calls_before_initialization_and_unsupported_api_versions() {
    let mut harness = Harness::new();
    harness.send(1, "models", json!({})).await;
    assert_eq!(harness.next().await["error"]["code"], "invalid_request");
    harness.send(2,"initialize",json!({"apiVersion":99,"pluginID":"fixture","pluginDirectory":"/plugin","dataDirectory":"/accounts","manifest":{}})).await;
    assert_eq!(harness.next().await["error"]["code"], "invalid_request");
}

#[tokio::test]
async fn fetch_streams_http_status_headers_and_bounded_base64_chunks() {
    let mut harness = Harness::new();
    harness.initialize().await;
    harness.send(2,"fetch",json!({"url":"https://provider.example/v1/chat/completions","method":"POST","headers":{},"credentials":{"key":"secret"},"body":STANDARD.encode(b"hello")})).await;
    let head = harness.next().await;
    assert_eq!(head["event"], "head");
    assert_eq!(head["status"], 429);
    assert_eq!(head["headers"]["retry-after"], "2");
    assert_eq!(head["headers"]["echo-length"], "5");
    let mut chunks = Vec::new();
    loop {
        let frame = harness.next().await;
        if frame["event"] == "done" {
            break;
        }
        assert_eq!(frame["event"], "chunk");
        let bytes = STANDARD.decode(frame["data"].as_str().unwrap()).unwrap();
        assert!(bytes.len() <= MAX_HTTP_CHUNK_BYTES);
        chunks.extend(bytes);
    }
    assert_eq!(chunks, vec![b'x'; 2 * MAX_HTTP_CHUNK_BYTES + 7]);
}

#[tokio::test]
async fn advertised_body_file_support_delivers_verified_binary_bytes_and_rejects_changes() {
    static NEXT_FILE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    struct FileGuard(std::path::PathBuf);
    impl Drop for FileGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    for tampered in [false, true] {
        let mut harness = Harness::new();
        harness.initialize().await;
        let file = FileGuard(std::env::temp_dir().join(format!(
            "nora-sdk-runtime-body-{}-{}",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        )));
        let bytes = b"\x00\xffbinary-body\r\n";
        std::fs::write(&file.0, bytes).unwrap();
        let descriptor = crate::RequestBodyFile::from_path(&file.0).unwrap();
        if tampered {
            let mut changed = bytes.to_vec();
            changed[0] = 1;
            std::fs::write(&file.0, changed).unwrap();
        }
        harness
            .send(
                2,
                "fetch",
                json!({"url":"body-file","method":"POST","headers":{},"bodyFile":descriptor}),
            )
            .await;
        let head = harness.next().await;
        if tampered {
            assert_eq!(head["error"]["code"], "invalid_request");
            assert!(!head.to_string().contains(file.0.to_str().unwrap()));
            continue;
        }
        assert_eq!(head["event"], "head");
        assert_eq!(head["status"], 200);
        let mut received = Vec::new();
        loop {
            let frame = harness.next().await;
            if frame["event"] == "done" {
                break;
            }
            assert_eq!(frame["event"], "chunk");
            received.extend(STANDARD.decode(frame["data"].as_str().unwrap()).unwrap());
        }
        assert_eq!(received, bytes);
    }
}

#[tokio::test]
async fn abort_drops_the_live_response_body_before_the_upstream_completes() {
    let mut harness = Harness::new();
    harness.initialize().await;
    harness
        .send(
            2,
            "fetch",
            json!({"url":"hold","method":"POST","headers":{}}),
        )
        .await;
    assert_eq!(harness.next().await["event"], "head");
    harness.send(3, "abort", json!({"id":2})).await;
    let cancellation = harness.next().await;
    assert_eq!(cancellation["id"], 2);
    assert_eq!(cancellation["error"]["code"], "cancelled");
    assert_eq!(
        harness.next().await,
        json!({"id":3,"result":{"aborted":true}})
    );
    assert!(harness.dropped.load(Ordering::Acquire));
}

#[tokio::test]
async fn body_errors_are_reported_after_the_partial_stream_without_done() {
    let mut harness = Harness::new();
    harness.initialize().await;
    harness
        .send(
            2,
            "fetch",
            json!({"url":"broken","method":"POST","headers":{}}),
        )
        .await;
    assert_eq!(harness.next().await["event"], "head");
    assert_eq!(harness.next().await["event"], "chunk");
    let error = harness.next().await;
    assert_eq!(error["error"]["message"], "upstream closed");
    assert!(error.get("event").is_none());
}

#[tokio::test]
async fn errors_keep_request_identity_and_do_not_corrupt_following_calls() {
    let mut harness = Harness::new();
    harness.initialize().await;
    harness
        .send(
            2,
            "fetch",
            json!({"url":"error","method":"POST","headers":{}}),
        )
        .await;
    let error = harness.next().await;
    assert_eq!(error["id"], 2);
    assert_eq!(error["error"]["code"], "authentication_required");
    harness.send(3, "unknown", json!({})).await;
    assert_eq!(
        harness.next().await["error"]["code"],
        "method_not_supported"
    );
    harness.send(4, "models", json!({})).await;
    assert!(harness.next().await.get("result").is_some());
}

#[tokio::test]
async fn oversized_reply_becomes_a_bounded_protocol_error() {
    let (writer, mut reader) = tokio::io::duplex(2 * MAX_RESPONSE_FRAME_BYTES);
    let task = tokio::spawn(async move {
        write_frame(
            &Arc::new(Mutex::new(writer)),
            json!({"id":42,"result":"x".repeat(MAX_RESPONSE_FRAME_BYTES)}),
        )
        .await
        .unwrap();
    });
    let mut bytes = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut bytes)
        .await
        .unwrap();
    task.await.unwrap();
    assert!(bytes.len() < MAX_RESPONSE_FRAME_BYTES);
    let frame: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(frame["id"], 42);
    assert_eq!(frame["error"]["code"], "response_too_large");
}

#[tokio::test]
async fn malformed_and_oversized_input_is_rejected() {
    let mut harness = Harness::new();
    harness.input.write_all(b"not JSON\n").await.unwrap();
    assert_eq!(harness.next().await["error"]["code"], "invalid_request");
    let bytes = vec![b'x'; MAX_REQUEST_BYTES + 1];
    let mut reader = BufReader::new(bytes.as_slice());
    assert_eq!(
        read_frame(&mut reader).await.unwrap_err().kind(),
        std::io::ErrorKind::InvalidData
    );
}

#[test]
fn request_body_decode_preserves_binary_data_and_rejects_invalid_base64() {
    let mut request:FetchRequest=serde_json::from_value(json!({"url":"https://provider.example","method":"POST","body":STANDARD.encode([0u8,255,128,42])})).unwrap();
    assert_eq!(request.decode_body().unwrap(), Some(vec![0, 255, 128, 42]));
    request.body = Some("not base64!".to_string());
    assert!(request.decode_body().is_err());
}

#[tokio::test]
async fn dropping_the_runtime_cancels_live_fetch_tasks() {
    let mut harness = Harness::new();
    harness.initialize().await;
    harness
        .send(
            2,
            "fetch",
            json!({"url":"hold","method":"POST","headers":{}}),
        )
        .await;
    assert_eq!(harness.next().await["event"], "head");
    let dropped = harness.dropped.clone();
    harness.task.abort();
    timeout(Duration::from_secs(2), async {
        while !dropped.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

async fn credentials_harness() -> Harness {
    let mut harness = Harness::new();
    harness.send(1,"initialize",json!({"apiVersion":2,"pluginID":"fixture","pluginDirectory":"/plugin","dataDirectory":"/accounts","manifest":{},"hostCapabilities":{"credentialUpdates":true}})).await;
    assert_eq!(harness.next().await["result"]["apiVersion"], 1);
    harness
}

#[tokio::test]
async fn renewed_credentials_wait_for_the_host_before_inference_continues() {
    let mut harness = credentials_harness().await;
    harness
        .send(2, "fetch", json!({"url":"renew","method":"POST"}))
        .await;
    let event = harness.next().await;
    assert_eq!(event["id"], 2);
    assert_eq!(event["event"], "credentials");
    assert_eq!(event["credentials"]["refresh"], "synthetic-new-refresh");
    let update = event["updateID"].as_u64().unwrap();
    harness
        .send(
            0,
            "credentialsSaved",
            json!({"id":99,"updateID":update,"success":true}),
        )
        .await;
    let mut line = String::new();
    assert!(timeout(
        Duration::from_millis(30),
        harness.output.read_line(&mut line)
    )
    .await
    .is_err());
    harness
        .send(
            0,
            "credentialsSaved",
            json!({"id":2,"updateID":update,"success":true}),
        )
        .await;
    let head = harness.next().await;
    assert_eq!(head["event"], "head");
    let body = harness.next().await;
    assert_eq!(body["event"], "chunk");
    assert!(!body.to_string().contains("synthetic-new"));
    assert_eq!(harness.next().await["event"], "done");
}

#[tokio::test]
async fn a_rejected_or_cancelled_credential_update_never_runs_inference() {
    for cancel in [false, true] {
        let mut harness = credentials_harness().await;
        harness
            .send(2, "fetch", json!({"url":"renew","method":"POST"}))
            .await;
        let event = harness.next().await;
        if cancel {
            harness.send(3, "abort", json!({"id":2})).await;
            assert_eq!(harness.next().await["error"]["code"], "cancelled");
            assert_eq!(harness.next().await["result"]["aborted"], true);
        } else {
            harness
                .send(
                    0,
                    "credentialsSaved",
                    json!({"id":2,"updateID":event["updateID"],"success":false}),
                )
                .await;
            let failure = harness.next().await;
            assert_eq!(failure["error"]["code"], "credential_update_failed");
            assert!(!failure.to_string().contains("synthetic-new"));
        }
        harness
            .send(
                0,
                "credentialsSaved",
                json!({"id":2,"updateID":event["updateID"],"success":true}),
            )
            .await;
        harness.send(4, "models", json!({})).await;
        assert_eq!(harness.next().await["id"], 4);
    }
}

#[tokio::test]
async fn credential_updates_require_a_compatible_host_and_an_active_request_scope() {
    assert_eq!(
        crate::save_credentials(json!({"access":"synthetic"}))
            .await
            .unwrap_err()
            .code,
        "method_not_supported"
    );
    let mut harness = Harness::new();
    harness.initialize().await;
    harness
        .send(2, "fetch", json!({"url":"renew","method":"POST"}))
        .await;
    let failure = harness.next().await;
    assert_eq!(failure["error"]["code"], "method_not_supported");
    assert!(failure.get("event").is_none());
}

#[tokio::test]
async fn metered_fetch_waits_without_blocking_protocol_reads_and_cancellation() {
    let mut host = Harness::new();
    host.send(1,"initialize",json!({"apiVersion":2,"pluginID":"fixture","pluginDirectory":"/plugin","dataDirectory":"/accounts","manifest":{},"hostCapabilities":{"httpRequestMeter":true}})).await;
    assert_eq!(
        host.next().await["result"]["capabilities"]["httpRequestMeter"],
        true
    );
    host.send(
        2,
        "fetch",
        json!({"url":"meter","method":"POST","headers":{}}),
    )
    .await;
    let first = host.next().await;
    assert_eq!(first["event"], "httpRequest");
    assert_eq!(first["id"], 2);
    host.send(3, "models", json!({})).await;
    assert_eq!(host.next().await["id"], 3);
    host.send(
        0,
        "httpRequestAllowed",
        json!({"id":2,"meterID":first["meterID"],"success":true}),
    )
    .await;
    let second = host.next().await;
    assert_eq!(second["event"], "httpRequest");
    assert_ne!(first["meterID"], second["meterID"]);
    host.send(4, "abort", json!({"id":2})).await;
    let mut cancelled = false;
    let mut terminal = false;
    let mut aborted = false;
    for _ in 0..3 {
        let frame = host.next().await;
        if frame["event"] == "httpRequestCancelled" {
            assert_eq!(frame["meterID"], second["meterID"]);
            cancelled = true;
        } else if frame["id"] == 2 {
            assert_eq!(frame["error"]["code"], "cancelled");
            terminal = true;
        } else if frame["id"] == 4 {
            assert_eq!(frame["result"]["aborted"], true);
            aborted = true;
        }
    }
    assert!(cancelled && terminal && aborted);
    host.send(5, "models", json!({})).await;
    assert_eq!(host.next().await["id"], 5);
}

struct SearchPlugin;
#[async_trait::async_trait]
impl ProviderPlugin for SearchPlugin {
    fn supports_web_search(&self) -> bool {
        true
    }
    async fn web_search(&self, context: CallContext, query: String) -> PluginResult<Value> {
        crate::before_http_request().await?;
        Ok(json!({"text":query,"sources":[{"url":"https://example.com","title":context.account}]}))
    }
}
#[tokio::test]
async fn search_api_queries_are_bounded_and_only_dispatched_for_an_implemented_feature() {
    assert!(dispatch(
        &SearchPlugin,
        "webSearch",
        json!({"query":"valid","account":"key"})
    )
    .await
    .is_ok());
    for query in [
        json!(""),
        json!(42),
        json!("q".repeat(16385)),
        json!("x\u{0000}y"),
    ] {
        assert_eq!(
            dispatch(&SearchPlugin, "webSearch", json!({"query":query}))
                .await
                .unwrap_err()
                .code,
            "invalid_request"
        );
    }
    assert_eq!(
        dispatch(
            &TestPlugin {
                dropped: Arc::default()
            },
            "webSearch",
            json!({"query":"valid"})
        )
        .await
        .unwrap_err()
        .code,
        "method_not_supported"
    );
}
#[tokio::test]
async fn search_api_http_requests_share_the_same_meter_protocol_as_fetch() {
    let (client, server) = tokio::io::duplex(65536);
    let (input, output) = tokio::io::split(server);
    let (reader, mut writer) = tokio::io::split(client);
    let task = tokio::spawn(serve_io(Arc::new(SearchPlugin), input, output));
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    writer.write_all(format!("{}\n",json!({"id":1,"method":"initialize","params":{"apiVersion":2,"pluginID":"search","pluginDirectory":"/plugin","dataDirectory":"/accounts","manifest":{},"hostCapabilities":{"httpRequestMeter":true}}})).as_bytes()).await.unwrap();
    reader.read_line(&mut line).await.unwrap();
    let initialized: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        initialized["result"]["capabilities"]["sdkFeatures"]["webSearch"],
        1
    );
    writer.write_all(format!("{}\n",json!({"id":2,"method":"webSearch","params":{"query":"verified","account":"api-key"}})).as_bytes()).await.unwrap();
    line.clear();
    timeout(Duration::from_secs(2), reader.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    let meter: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(meter["event"], "httpRequest");
    assert_eq!(meter["id"], 2);
    writer.write_all(format!("{}\n",json!({"id":0,"method":"httpRequestAllowed","params":{"id":2,"meterID":meter["meterID"],"success":true}})).as_bytes()).await.unwrap();
    line.clear();
    timeout(Duration::from_secs(2), reader.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    let result: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(result["result"]["text"], "verified");
    assert_eq!(result["result"]["sources"][0]["title"], "api-key");
    task.abort();
}

struct AlphaSearchPlugin;
#[async_trait::async_trait]
impl ProviderPlugin for AlphaSearchPlugin {
    fn supports_alpha_search(&self) -> bool {
        true
    }
    async fn alpha_search(
        &self,
        context: CallContext,
        request: Value,
        headers: std::collections::BTreeMap<String, String>,
    ) -> PluginResult<Value> {
        crate::before_http_request().await?;
        Ok(
            json!({"status":200,"body":{"request":request,"headers":headers,"account":context.account,"encrypted_output":"opaque-vendor-state"}}),
        )
    }
}

#[tokio::test]
async fn alpha_search_preserves_vendor_commands_and_requires_an_implemented_feature() {
    let request = json!({"model":"current","commands":{"open":[{"ref":"opaque"}]},"unknown_future_command":true});
    let result = dispatch(
        &AlphaSearchPlugin,
        "alphaSearch",
        json!({"request":request,"headers":{"session-id":"conversation"},"account":"chosen"}),
    )
    .await
    .unwrap();
    assert_eq!(result["body"]["request"], request);
    assert_eq!(result["body"]["headers"]["session-id"], "conversation");
    assert_eq!(result["body"]["account"], "chosen");
    assert_eq!(result["body"]["encrypted_output"], "opaque-vendor-state");
    for invalid in [json!(null), json!([]), json!(42)] {
        assert_eq!(
            dispatch(
                &AlphaSearchPlugin,
                "alphaSearch",
                json!({"request":invalid})
            )
            .await
            .unwrap_err()
            .code,
            "invalid_request"
        );
    }
    assert_eq!(
        dispatch(&SearchPlugin, "alphaSearch", json!({"request":request}))
            .await
            .unwrap_err()
            .code,
        "method_not_supported"
    );
}

#[tokio::test]
async fn alpha_search_negotiates_capability_and_waits_for_http_meter_before_answering() {
    let (client, server) = tokio::io::duplex(65536);
    let (input, output) = tokio::io::split(server);
    let (reader, mut writer) = tokio::io::split(client);
    let task = tokio::spawn(serve_io(Arc::new(AlphaSearchPlugin), input, output));
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    writer.write_all(format!("{}\n",json!({"id":1,"method":"initialize","params":{"apiVersion":2,"pluginID":"openai","pluginDirectory":"/plugin","dataDirectory":"/accounts","manifest":{},"hostCapabilities":{"httpRequestMeter":true}}})).as_bytes()).await.unwrap();
    reader.read_line(&mut line).await.unwrap();
    let initialization: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        initialization["result"]["capabilities"]["sdkFeatures"]["alphaSearch"],
        1
    );
    writer.write_all(format!("{}\n",json!({"id":2,"method":"alphaSearch","params":{"request":{"model":"chat","commands":{}},"headers":{}}})).as_bytes()).await.unwrap();
    line.clear();
    timeout(Duration::from_secs(2), reader.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    let meter: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(meter["event"], "httpRequest");
    assert_eq!(meter["id"], 2);
    writer.write_all(format!("{}\n",json!({"id":0,"method":"httpRequestAllowed","params":{"id":2,"meterID":meter["meterID"],"success":true}})).as_bytes()).await.unwrap();
    line.clear();
    timeout(Duration::from_secs(2), reader.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    let answer: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(answer["id"], 2);
    assert_eq!(
        answer["result"]["body"]["encrypted_output"],
        "opaque-vendor-state"
    );
    task.abort();
}

struct LearningPlugin;
#[async_trait::async_trait]
impl ProviderPlugin for LearningPlugin {
    fn supports_learned_usage(&self) -> bool {
        true
    }
    async fn learned_usage(&self, context: CallContext) -> PluginResult<Option<Value>> {
        if context.account == "unknown" {
            return Ok(None);
        }
        Ok(Some(
            json!({"observedAt":"2026-10-26T00:00:00Z","items":[{"id":"five_hour","usedPercent":42,"windowSeconds":18000,"countsForRouting":true}],"account":{"id":context.account}}),
        ))
    }
}
#[tokio::test]
async fn learned_allowance_is_optional_bound_to_the_selected_account_and_feature_gated() {
    let known = dispatch(
        &LearningPlugin,
        "learnedUsage",
        json!({"account":"selected"}),
    )
    .await
    .unwrap();
    assert_eq!(known["items"][0]["usedPercent"], 42);
    assert_eq!(known["account"]["id"], "selected");
    assert!(dispatch(
        &LearningPlugin,
        "learnedUsage",
        json!({"account":"unknown"})
    )
    .await
    .unwrap()
    .is_null());
    assert_eq!(
        dispatch(&SearchPlugin, "learnedUsage", json!({"account":"selected"}))
            .await
            .unwrap_err()
            .code,
        "method_not_supported"
    );
}
#[tokio::test]
async fn learned_allowance_negotiates_capability_and_answers_without_http_permission_events() {
    let (client, server) = tokio::io::duplex(65536);
    let (input, output) = tokio::io::split(server);
    let (reader, mut writer) = tokio::io::split(client);
    let task = tokio::spawn(serve_io(Arc::new(LearningPlugin), input, output));
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    writer.write_all(format!("{}\n",json!({"id":1,"method":"initialize","params":{"apiVersion":2,"pluginID":"fixture","pluginDirectory":"/plugin","dataDirectory":"/accounts","manifest":{},"hostCapabilities":{"httpRequestMeter":true}}})).as_bytes()).await.unwrap();
    reader.read_line(&mut line).await.unwrap();
    let init: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        init["result"]["capabilities"]["sdkFeatures"]["learningUsage"],
        1
    );
    writer
        .write_all(
            format!(
                "{}\n",
                json!({"id":2,"method":"learnedUsage","params":{"account":"selected"}})
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    line.clear();
    timeout(Duration::from_secs(2), reader.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    let reply: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(reply["id"], 2);
    assert!(reply.get("event").is_none());
    assert_eq!(reply["result"]["items"][0]["usedPercent"], 42);
    task.abort();
}
