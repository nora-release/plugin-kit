use nora_plugin_runtime_sdk::protocol::{Request, Response, PROTOCOL_VERSION};
use nora_plugin_runtime_sdk::transport::{read_frame, write_frame};
use serde_json::{json, Value};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::BufReader;
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

struct Fixture {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

impl Fixture {
    fn new() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_nora-plugin-runtime-sdk-fixture"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        Self {
            input: child.stdin.take().unwrap(),
            output: BufReader::new(child.stdout.take().unwrap()),
            child,
        }
    }

    async fn send(&mut self, value: Value) {
        write_frame(&mut self.input, &value, 1024 * 1024)
            .await
            .unwrap();
    }

    async fn read(&mut self) -> Value {
        tokio::time::timeout(
            Duration::from_secs(3),
            read_frame(&mut self.output, 1024 * 1024),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap()
    }

    async fn initialize(&mut self, config: Value, grants: Vec<&str>) {
        self.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":PROTOCOL_VERSION,"pluginId":"fixture","config":config,"grantedServices":grants}})).await;
    }

    async fn shutdown(mut self) {
        self.send(serde_json::to_value(Request::new(100, "shutdown", Value::Null)).unwrap())
            .await;
        let response: Response = serde_json::from_value(self.read().await).unwrap();
        assert!(response.is_valid());
        assert_eq!(response.result, Some(Value::Null));
        assert!(
            tokio::time::timeout(Duration::from_secs(3), self.child.wait())
                .await
                .unwrap()
                .unwrap()
                .success()
        );
    }
}

#[tokio::test]
async fn a_real_rust_sdk_process_initializes_calls_tools_and_calls_host_services() {
    let mut fixture = Fixture::new();
    fixture.initialize(json!({}), vec!["fixture.echo"]).await;
    assert_eq!(fixture.read().await["result"]["tools"][0]["name"], "echo");
    fixture.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"echo","arguments":{"hello":"world"}}})).await;
    assert!(fixture.read().await["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("world"));

    fixture.send(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"service","arguments":{"service":true}}})).await;
    let reverse = fixture.read().await;
    assert_eq!(reverse["method"], "services/call");
    assert_eq!(reverse["params"]["parentRequestId"], 3);
    fixture
        .send(json!({"jsonrpc":"2.0","id":reverse["id"],"result":{"fromHost":true}}))
        .await;
    assert!(fixture.read().await["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("fromHost"));
    fixture.shutdown().await;
}

#[tokio::test]
async fn initialization_can_call_only_the_services_granted_by_the_host() {
    let mut fixture = Fixture::new();
    fixture
        .initialize(json!({"initializeService":true}), vec!["fixture.echo"])
        .await;
    let reverse = fixture.read().await;
    assert_eq!(reverse["method"], "services/call");
    assert_eq!(reverse["params"]["parentRequestId"], 1);
    fixture
        .send(json!({"jsonrpc":"2.0","id":reverse["id"],"result":true}))
        .await;
    assert_eq!(
        fixture.read().await["result"]["protocolVersion"],
        PROTOCOL_VERSION
    );
    fixture.shutdown().await;

    let mut denied = Fixture::new();
    denied
        .initialize(json!({"initializeService":true}), Vec::new())
        .await;
    assert_eq!(denied.read().await["error"]["code"], -32003);
    denied.shutdown().await;
}

#[tokio::test]
async fn cancellation_ends_one_request_and_the_process_remains_available() {
    let mut fixture = Fixture::new();
    fixture.initialize(json!({}), Vec::new()).await;
    fixture.read().await;
    fixture.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"wait","arguments":{}}})).await;
    fixture
        .send(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":2}}))
        .await;
    assert_eq!(fixture.read().await["error"]["code"], -32800);
    fixture.send(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"echo","arguments":{}}})).await;
    assert_eq!(fixture.read().await["id"], 3);
    fixture.shutdown().await;
}

#[tokio::test]
async fn only_declared_services_are_called_and_reverse_calls_use_the_actual_inbound_id() {
    let mut fixture = Fixture::new();
    fixture.initialize(json!({}), vec!["fixture.echo"]).await;
    assert_eq!(
        fixture.read().await["result"]["services"],
        json!(["example.echo"])
    );
    fixture.send(json!({"jsonrpc":"2.0","id":2,"method":"services/call","params":{"service":"unknown","method":"echo","arguments":{}}})).await;
    assert_eq!(fixture.read().await["error"]["code"], -32601);
    fixture.send(json!({"jsonrpc":"2.0","id":3,"method":"services/call","params":{"service":"example.echo","method":"echo","arguments":null}})).await;
    let response: Response = serde_json::from_value(fixture.read().await).unwrap();
    assert!(response.is_valid());
    assert_eq!(response.result, Some(Value::Null));
    fixture.send(json!({"jsonrpc":"2.0","id":4,"method":"services/call","params":{"service":"example.echo","method":"reverse","arguments":{"fromService":true},"parentRequestId":99999}})).await;
    let reverse = fixture.read().await;
    assert_eq!(reverse["params"]["parentRequestId"], 4);
    fixture
        .send(json!({"jsonrpc":"2.0","id":reverse["id"],"result":{"fromHost":true}}))
        .await;
    assert_eq!(fixture.read().await["result"], json!({"fromHost":true}));
    fixture.shutdown().await;
}

#[tokio::test]
async fn cancellation_cleans_one_reverse_call_and_ignores_its_late_reply() {
    let mut fixture = Fixture::new();
    fixture.initialize(json!({}), vec!["fixture.echo"]).await;
    fixture.read().await;
    fixture.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"service","arguments":{}}})).await;
    let first = fixture.read().await;
    assert_eq!(first["params"]["parentRequestId"], 2);
    fixture.send(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"service","arguments":{}}})).await;
    let second = fixture.read().await;
    assert_eq!(second["params"]["parentRequestId"], 3);
    fixture
        .send(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":2}}))
        .await;
    let frames = [fixture.read().await, fixture.read().await];
    assert!(frames
        .iter()
        .any(|frame| frame["method"] == "notifications/cancelled"
            && frame["params"]["requestId"] == first["id"]));
    assert!(frames
        .iter()
        .any(|frame| frame["id"] == 2 && frame["error"]["code"] == -32800));
    fixture
        .send(json!({"jsonrpc":"2.0","id":first["id"],"result":{"late":true}}))
        .await;
    fixture
        .send(json!({"jsonrpc":"2.0","id":second["id"],"result":{"second":true}}))
        .await;
    let response = fixture.read().await;
    assert_eq!(response["id"], 3);
    assert!(response["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("second"));
    fixture.send(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"echo","arguments":{}}})).await;
    assert_eq!(fixture.read().await["id"], 4);
    fixture.shutdown().await;
}

/// A host for protocol 2: a stream table whose frames go to the plugin's
/// stdin, and a reader that hands the plugin's stream frames to the table
/// and everything else to `replies`.
struct StreamingHost {
    child: Child,
    table: std::sync::Arc<nora_plugin_runtime_sdk::streams::StreamTable>,
    to_plugin: tokio::sync::mpsc::Sender<Value>,
    replies: tokio::sync::mpsc::UnboundedReceiver<Value>,
}

impl StreamingHost {
    async fn start(version: u32) -> Self {
        use nora_plugin_runtime_sdk::streams::{Side, StreamTable};
        let Fixture {
            child,
            mut input,
            mut output,
        } = Fixture::new();
        let (to_plugin, mut frames) = tokio::sync::mpsc::channel::<Value>(256);
        tokio::spawn(async move {
            while let Some(frame) = frames.recv().await {
                if write_frame(&mut input, &frame, 1024 * 1024).await.is_err() {
                    return;
                }
            }
        });
        let table = StreamTable::new(
            Side::Host,
            nora_plugin_runtime_sdk::protocol::MIN_STREAM_WINDOW,
            std::sync::Arc::new(to_plugin.clone()),
        );
        let (reply, replies) = tokio::sync::mpsc::unbounded_channel();
        let routed = std::sync::Arc::clone(&table);
        tokio::spawn(async move {
            while let Ok(Some(frame)) = read_frame(&mut output, 1024 * 1024).await {
                match frame["method"].as_str() {
                    Some("streams/data") => routed.on_data(frame["params"].clone()).unwrap(),
                    Some("streams/credit") => routed.on_credit(frame["params"].clone()).unwrap(),
                    _ => {
                        let _ = reply.send(frame);
                    }
                }
            }
        });
        let mut host = Self {
            child,
            table,
            to_plugin,
            replies,
        };
        let mut params =
            json!({"protocolVersion": version, "pluginId": "fixture", "grantedServices": []});
        if version == 2 {
            params["streamWindow"] = json!(host.table.read_window());
        }
        host.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":params}))
            .await;
        let initialized = host.reply().await;
        assert_eq!(initialized["result"]["protocolVersion"], version);
        host.table
            .set_write_window(initialized["result"]["streamWindow"].as_u64())
            .unwrap();
        host
    }

    async fn send(&self, frame: Value) {
        self.to_plugin.send(frame).await.unwrap();
    }

    async fn reply(&mut self) -> Value {
        tokio::time::timeout(Duration::from_secs(5), self.replies.recv())
            .await
            .unwrap()
            .unwrap()
    }
}

const CALL: fn(u64) -> nora_plugin_runtime_sdk::streams::CallKey =
    |id| nora_plugin_runtime_sdk::streams::CallKey { local: true, id };

#[tokio::test]
async fn protocol_two_reads_a_body_answers_then_streams_one_larger_than_any_frame() {
    let mut host = StreamingHost::start(2).await;
    let input = host.table.open_writer(CALL(2)).unwrap();
    let mut output = host.table.open_reader(CALL(2)).unwrap();
    host.send(
        json!({"jsonrpc":"2.0","id":2,"method":"services/call","params":{
            "service":"example.echo","method":"copy","arguments":{},
            "inputStream": input.id(), "outputStream": output.id(),
        }}),
    )
    .await;
    let body: Vec<u8> = (0..3_000_000u32).map(|i| (i % 253) as u8).collect();
    let expected = body.clone();
    let writing = tokio::spawn(async move {
        let mut input = input;
        input.write(&body).await.unwrap();
        input.finish().await.unwrap();
    });
    // the head comes once the input is in, before any of the body
    assert_eq!(host.reply().await["result"], json!({"head": true}));
    host.table.close_writers(
        CALL(2),
        &nora_plugin_runtime_sdk::protocol::RpcError::new(-32000, "answered"),
    );
    let copied = tokio::time::timeout(Duration::from_secs(20), output.read_to_end(4_000_000))
        .await
        .expect("the copy stalled")
        .unwrap();
    writing.await.unwrap();
    assert_eq!(copied.len(), expected.len());
    assert!(copied == expected);
}

#[tokio::test]
async fn a_plugin_that_gives_up_mid_stream_ends_it_with_its_error() {
    let mut host = StreamingHost::start(2).await;
    let mut output = host.table.open_reader(CALL(2)).unwrap();
    host.send(
        json!({"jsonrpc":"2.0","id":2,"method":"services/call","params":{
            "service":"example.echo","method":"spill","arguments":{"bytes": 100_000, "fail": true},
            "outputStream": output.id(),
        }}),
    )
    .await;
    let mut read = 0;
    let error = loop {
        match output.read().await.unwrap() {
            Ok(chunk) => read += chunk.len(),
            Err(error) => break error,
        }
    };
    assert_eq!((read, error.code), (100_000, -32050));
    assert_eq!(host.reply().await["result"]["wrote"], 100_000);
}

#[tokio::test]
async fn a_cancelled_call_stops_its_streams_and_the_plugin_serves_on() {
    let mut host = StreamingHost::start(2).await;
    let mut input = host.table.open_writer(CALL(2)).unwrap();
    let output = host.table.open_reader(CALL(2)).unwrap();
    host.send(
        json!({"jsonrpc":"2.0","id":2,"method":"services/call","params":{
            "service":"example.echo","method":"copy","arguments":{},
            "inputStream": input.id(), "outputStream": output.id(),
        }}),
    )
    .await;
    // part of the input, then the call is cancelled mid-way
    input.write(&[1u8; 10_000]).await.unwrap();
    host.send(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":2}}))
        .await;
    assert_eq!(host.reply().await["error"]["code"], -32800);
    host.table.close_call(
        CALL(2),
        &nora_plugin_runtime_sdk::protocol::RpcError::new(-32800, "cancelled"),
    );
    drop((input, output));
    host.send(
        json!({"jsonrpc":"2.0","id":3,"method":"services/call","params":{
        "service":"example.echo","method":"echo","arguments":{"still":"here"}}}),
    )
    .await;
    assert_eq!(host.reply().await["result"]["still"], "here");
}

#[tokio::test]
async fn protocol_one_takes_no_streams() {
    let mut host = StreamingHost::start(1).await;
    host.send(
        json!({"jsonrpc":"2.0","id":2,"method":"services/call","params":{
        "service":"example.echo","method":"spill","arguments":{},"outputStream": 1}}),
    )
    .await;
    assert_eq!(host.reply().await["error"]["code"], -32602);
    // a stream frame in a protocol 1 session ends it
    host.send(json!({"jsonrpc":"2.0","method":"streams/credit","params":{"stream":1,"bytes":1}}))
        .await;
    let status = tokio::time::timeout(Duration::from_secs(3), host.child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(!status.success());
}
