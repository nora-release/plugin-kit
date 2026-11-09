use super::*;
use crate::protocol::{CALL_SERVICE, INITIALIZE, STREAM_DATA};
use crate::{serve_io, ServerOptions};
use base64::Engine;
use serde_json::json;
use std::time::{Duration, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines};

/// A provider answering what the router asks, as a plugin would.
struct Acme;

#[derive(serde::Deserialize)]
struct Credentials {
    token: String,
}

impl Provider for Acme {
    async fn sign_in(&self, request: SignIn) -> Result<Step, RpcError> {
        if request.way == "key" {
            let key = request.inputs.get("key").cloned().unwrap_or_default();
            return Ok(Step::Account(Account {
                name: "key".into(),
                credentials: json!({"token": key}),
                expires_at: None,
            }));
        }
        Ok(Step::Open {
            url: "https://login.acme.test".into(),
            code: Some("ABCD".into()),
            session: "s1".into(),
        })
    }

    async fn sign_in_finish(&self, request: SignInFinish) -> Result<Step, RpcError> {
        if request.code.is_empty() {
            return Ok(Step::Waiting);
        }
        Ok(Step::Account(Account {
            name: "ada@acme.test".into(),
            credentials: json!({"token": request.code}),
            expires_at: Some(rfc3339(UNIX_EPOCH + Duration::from_secs(86_400))),
        }))
    }

    async fn models(&self, question: Question) -> Result<Vec<Model>, RpcError> {
        let token = question.credentials::<Credentials>()?.token;
        if token == "expired" {
            return Err(sign_in_needed("the token expired"));
        }
        if token == "busy" {
            return Err(rate_limited("slow down", Some(Duration::from_secs(30))));
        }
        let mut models = vec![Model::new("acme-large")];
        if !question.option("freeOnly") {
            models.push(Model {
                reasoning: true,
                ..Model::new("acme-pro")
            });
        }
        Ok(models)
    }

    async fn fetch(&self, mut request: Fetch) -> Result<FetchHead, RpcError> {
        let body = request.body.read_to_end(1 << 20).await?;
        let mut response = request.response;
        let echo = format!(
            "{} {} {}",
            request.method,
            request.url,
            String::from_utf8_lossy(&body)
        );
        tokio::spawn(async move {
            let _ = response.write(echo.as_bytes()).await;
            let _ = response.finish().await;
        });
        Ok(FetchHead {
            status: 201,
            headers: [("content-type".into(), "text/plain".into())].into(),
            ..FetchHead::default()
        })
    }
}

/// The router's side of the pipe: JSON lines in and out.
struct Host {
    writer: DuplexStream,
    lines: Lines<BufReader<DuplexStream>>,
    next: u64,
}

impl Host {
    async fn start() -> Self {
        let (writer, plugin_input) = tokio::io::duplex(1 << 20);
        let (plugin_output, reader) = tokio::io::duplex(1 << 20);
        tokio::spawn(serve_io(
            ProviderPlugin(Arc::new(Acme)),
            plugin_input,
            plugin_output,
            ServerOptions::default(),
        ));
        let mut host = Self {
            writer,
            lines: BufReader::new(reader).lines(),
            next: 0,
        };
        let result = host
            .call(
                INITIALIZE,
                json!({"protocolVersion": 2, "pluginId": "example.acme", "streamWindow": 65536}),
            )
            .await
            .unwrap();
        assert_eq!(result["services"], json!([SERVICE]));
        host
    }

    async fn send(&mut self, frame: Value) {
        let mut line = serde_json::to_vec(&frame).unwrap();
        line.push(b'\n');
        self.writer.write_all(&line).await.unwrap();
    }

    async fn frame(&mut self) -> Value {
        let line = self.lines.next_line().await.unwrap().unwrap();
        serde_json::from_str(&line).unwrap()
    }

    async fn call(&mut self, method: &str, params: Value) -> Result<Value, Value> {
        self.next += 1;
        let id = self.next;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await;
        loop {
            let frame = self.frame().await;
            if frame["id"] == id {
                return match frame.get("error") {
                    Some(error) => Err(error.clone()),
                    None => Ok(frame["result"].clone()),
                };
            }
        }
    }

    async fn ask(&mut self, method: &str, arguments: Value) -> Result<Value, Value> {
        self.call(
            CALL_SERVICE,
            json!({"service": SERVICE, "method": method, "arguments": arguments}),
        )
        .await
    }
}

#[tokio::test]
async fn sign_ins_answer_the_steps_the_router_reads() {
    let mut host = Host::start().await;
    let key = host
        .ask(
            "provider/signin",
            json!({"provider": "acme", "way": "key", "inputs": {"key": "sk-1"}}),
        )
        .await
        .unwrap();
    assert_eq!(
        key,
        json!({"account": {"name": "key", "credentials": {"token": "sk-1"}}})
    );

    let open = host
        .ask(
            "provider/signin",
            json!({"provider": "acme", "way": "browser", "inputs": {}}),
        )
        .await
        .unwrap();
    assert_eq!(
        open,
        json!({"open": "https://login.acme.test", "code": "ABCD", "session": "s1"})
    );
    let waiting = host
        .ask(
            "provider/signin/finish",
            json!({"provider": "acme", "session": "s1", "code": ""}),
        )
        .await
        .unwrap();
    assert_eq!(waiting, json!({"waiting": true}));
    let done = host
        .ask(
            "provider/signin/finish",
            json!({"provider": "acme", "session": "s1", "code": "t1"}),
        )
        .await
        .unwrap();
    assert_eq!(done["account"]["expiresAt"], "1970-01-02T00:00:00Z");
}

#[tokio::test]
async fn models_follow_the_credentials_and_options_and_errors_carry_their_codes() {
    let mut host = Host::start().await;
    let models = host
        .ask("provider/models", json!({"provider": "acme", "credentials": {"token": "t"}, "options": {"freeOnly": true}}))
        .await
        .unwrap();
    assert_eq!(models, json!([{"id": "acme-large"}]));
    let all = host
        .ask(
            "provider/models",
            json!({"provider": "acme", "credentials": {"token": "t"}}),
        )
        .await
        .unwrap();
    assert_eq!(all[1], json!({"id": "acme-pro", "reasoning": true}));

    let expired = host
        .ask(
            "provider/models",
            json!({"provider": "acme", "credentials": {"token": "expired"}}),
        )
        .await
        .unwrap_err();
    assert_eq!(expired["code"], SIGN_IN_NEEDED);
    let busy = host
        .ask(
            "provider/models",
            json!({"provider": "acme", "credentials": {"token": "busy"}}),
        )
        .await
        .unwrap_err();
    assert_eq!(
        (busy["code"].clone(), busy["data"]["retryAfter"].clone()),
        (json!(RATE_LIMITED), json!(30.0))
    );

    let unanswered = host
        .ask(
            "provider/usage",
            json!({"provider": "acme", "credentials": {"token": "t"}}),
        )
        .await
        .unwrap_err();
    assert_eq!(unanswered["code"], -32601);
}

#[tokio::test]
async fn a_fetch_reads_the_request_body_answers_its_head_then_streams_the_response() {
    let mut host = Host::start().await;
    let body = base64::engine::general_purpose::STANDARD.encode(b"{\"hi\":1}");
    host.send(json!({"jsonrpc": "2.0", "id": 99, "method": CALL_SERVICE, "params": {
        "service": SERVICE, "method": "provider/fetch", "inputStream": 1, "outputStream": 3,
        "arguments": {"provider": "acme", "credentials": {"token": "t"},
                      "request": {"method": "POST", "url": "https://api.acme.test/v1/messages", "headers": {"x": "y"}}},
    }}))
    .await;
    host.send(json!({"jsonrpc": "2.0", "method": STREAM_DATA, "params": {"stream": 1, "data": body, "end": true}}))
        .await;
    let mut head = None;
    let mut response = Vec::new();
    loop {
        let frame = host.frame().await;
        if frame["id"] == 99 {
            head = Some(frame["result"].clone());
        } else if frame["method"] == STREAM_DATA && frame["params"]["stream"] == 3 {
            if let Some(data) = frame["params"]["data"].as_str() {
                response.extend(
                    base64::engine::general_purpose::STANDARD
                        .decode(data)
                        .unwrap(),
                );
            }
            if frame["params"]["end"] == true {
                break;
            }
        }
    }
    assert_eq!(
        head.unwrap(),
        json!({"status": 201, "headers": {"content-type": "text/plain"}})
    );
    assert_eq!(
        String::from_utf8(response).unwrap(),
        "POST https://api.acme.test/v1/messages {\"hi\":1}"
    );
}

#[test]
fn rfc3339_writes_utc_dates() {
    assert_eq!(rfc3339(UNIX_EPOCH), "1970-01-01T00:00:00Z");
    assert_eq!(
        rfc3339(UNIX_EPOCH + Duration::from_secs(1_772_323_200 + 3_723)),
        "2026-03-01T01:02:03Z"
    );
    assert_eq!(
        rfc3339(UNIX_EPOCH + Duration::from_secs(951_782_400)),
        "2000-02-29T00:00:00Z"
    );
}
