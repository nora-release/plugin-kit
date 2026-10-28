use super::*;
use std::sync::atomic::AtomicUsize;
use tokio::io::{AsyncBufReadExt, BufReader, DuplexStream, ReadHalf, WriteHalf};
use tokio::time::{timeout, Duration};
struct Harness {
    writer: Arc<AsyncMutex<WriteHalf<DuplexStream>>>,
    reader: BufReader<ReadHalf<DuplexStream>>,
    acks: Arc<Acknowledgments>,
    enabled: Arc<AtomicBool>,
}
impl Harness {
    fn new(enabled: bool) -> Self {
        let (a, b) = tokio::io::duplex(8192);
        let (_, writer) = tokio::io::split(a);
        let (reader, _) = tokio::io::split(b);
        Self {
            writer: Arc::new(AsyncMutex::new(writer)),
            reader: BufReader::new(reader),
            acks: Arc::default(),
            enabled: Arc::new(AtomicBool::new(enabled)),
        }
    }
    async fn next(&mut self) -> Value {
        let mut line = String::new();
        timeout(Duration::from_secs(2), self.reader.read_line(&mut line))
            .await
            .unwrap()
            .unwrap();
        serde_json::from_str(&line).unwrap()
    }
    fn answer(&self, frame: &Value, success: bool) {
        self.acks
            .receive(&json!({"id":frame["id"],"meterID":frame["meterID"],"success":success}));
    }
}
#[tokio::test]
async fn every_attempt_waits_for_its_own_acknowledgment_and_denials_do_not_send() {
    let mut host = Harness::new(true);
    let sent = Arc::new(AtomicUsize::new(0));
    let seen = sent.clone();
    let writer = host.writer.clone();
    let acks = host.acks.clone();
    let enabled = host.enabled.clone();
    let task = tokio::spawn(scope(7, writer, acks, enabled, async move {
        for _ in 0..2 {
            before_http_request().await?;
            seen.fetch_add(1, Ordering::SeqCst);
        }
        Ok::<_, PluginError>(())
    }));
    let first = host.next().await;
    assert_eq!(first["event"], "httpRequest");
    assert_eq!(sent.load(Ordering::SeqCst), 0);
    host.answer(&first, true);
    let second = host.next().await;
    assert_ne!(first["meterID"], second["meterID"]);
    assert_eq!(sent.load(Ordering::SeqCst), 1);
    host.answer(&second, false);
    assert_eq!(
        task.await.unwrap().unwrap_err().code,
        "http_request_refused"
    );
    assert_eq!(sent.load(Ordering::SeqCst), 1);
    assert!(host.acks.pending.lock().unwrap().is_empty());
}
#[tokio::test]
async fn cancelling_one_wait_retracts_only_that_http_attempt() {
    let mut host = Harness::new(true);
    let task = tokio::spawn(scope(
        7,
        host.writer.clone(),
        host.acks.clone(),
        host.enabled.clone(),
        before_http_request(),
    ));
    let frame = host.next().await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let cancelled = host.next().await;
    assert_eq!(cancelled["event"], "httpRequestCancelled");
    assert_eq!(cancelled["id"], frame["id"]);
    assert_eq!(cancelled["meterID"], frame["meterID"]);
    assert!(host.acks.pending.lock().unwrap().is_empty());
    let other = tokio::spawn(scope(
        8,
        host.writer.clone(),
        host.acks.clone(),
        host.enabled.clone(),
        before_http_request(),
    ));
    let next = host.next().await;
    assert_eq!(next["id"], 8);
    host.answer(&frame, true);
    assert!(!other.is_finished());
    host.answer(&next, true);
    other.await.unwrap().unwrap();
}
#[tokio::test]
async fn inherited_background_context_yields_to_the_current_fetch() {
    let mut host = Harness::new(true);
    let background = scope(
        1,
        host.writer.clone(),
        host.acks.clone(),
        host.enabled.clone(),
        async { RequestMeterContext::capture() },
    )
    .await;
    let inherited = background.clone();
    let task = tokio::spawn(async move { inherited.scope(before_http_request()).await });
    let event = host.next().await;
    assert_eq!(event["id"], 1);
    host.answer(&event, true);
    task.await.unwrap().unwrap();
    let task = tokio::spawn(scope(
        2,
        host.writer.clone(),
        host.acks.clone(),
        host.enabled.clone(),
        async move { background.scope(before_http_request()).await },
    ));
    let event = host.next().await;
    assert_eq!(event["id"], 2);
    host.answer(&event, true);
    task.await.unwrap().unwrap();
}
#[tokio::test]
async fn cleanup_context_allows_exactly_one_explicitly_marked_attempt() {
    let mut host = Harness::new(true);
    let context = scope(
        3,
        host.writer.clone(),
        host.acks.clone(),
        host.enabled.clone(),
        async { RequestMeterContext::capture() },
    )
    .await;
    let task = tokio::spawn(async move {
        context
            .cleanup_scope(async {
                before_http_request().await?;
                before_http_request().await
            })
            .await
    });
    let frame = host.next().await;
    assert_eq!(frame["event"], "httpRequestCleanup");
    host.answer(&frame, true);
    assert!(task.await.unwrap().is_err());
}
#[tokio::test]
async fn old_hosts_and_calls_outside_a_fetch_do_not_wait_for_an_unknown_protocol() {
    before_http_request().await.unwrap();
    let host = Harness::new(false);
    scope(
        4,
        host.writer,
        host.acks,
        host.enabled,
        before_http_request(),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn real_http_send_starts_only_after_the_host_permission() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let accepted = Arc::new(AtomicUsize::new(0));
    let received = accepted.clone();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        received.fetch_add(1, Ordering::SeqCst);
        let mut bytes = [0u8; 8192];
        socket.read(&mut bytes).await.unwrap();
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
    });
    let mut host = Harness::new(true);
    let task = tokio::spawn(scope(
        9,
        host.writer.clone(),
        host.acks.clone(),
        host.enabled.clone(),
        async move {
            crate::send_http(crate::http_client(None)?.get(format!("http://{address}/"))).await
        },
    ));
    let frame = host.next().await;
    assert_eq!(accepted.load(Ordering::SeqCst), 0);
    host.answer(&frame, true);
    assert_eq!(task.await.unwrap().unwrap().status(), 200);
    server.await.unwrap();
    assert_eq!(accepted.load(Ordering::SeqCst), 1);
}
