use crate::transport::{read_frame, read_frame_buffered};
use serde_json::json;
use std::time::Duration;
use tokio::io::{AsyncWriteExt, BufReader};

#[tokio::test]
async fn an_oversized_unterminated_frame_is_rejected_without_unbounded_allocation() {
    let bytes = vec![b'x'; 129];
    let mut reader = BufReader::new(bytes.as_slice());
    let error = read_frame(&mut reader, 128).await.unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
}

#[tokio::test]
async fn a_cancelled_frame_read_preserves_the_consumed_prefix() {
    let (mut writer, input) = tokio::io::duplex(256);
    writer
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":")
        .await
        .unwrap();
    let mut reader = BufReader::new(input);
    let mut partial = Vec::new();
    assert!(tokio::time::timeout(
        Duration::from_millis(10),
        read_frame_buffered(&mut reader, &mut partial, 256)
    )
    .await
    .is_err());
    assert!(!partial.is_empty());
    writer.write_all(b"1,\"result\":null}\n").await.unwrap();
    assert_eq!(
        read_frame_buffered(&mut reader, &mut partial, 256)
            .await
            .unwrap(),
        Some(json!({"jsonrpc":"2.0","id":1,"result":null}))
    );
    assert!(partial.is_empty());
}

#[tokio::test]
async fn an_incomplete_frame_is_not_accepted_as_a_complete_message() {
    let mut reader = BufReader::new(b"{\"id\":1}".as_slice());
    assert_eq!(
        read_frame(&mut reader, 128).await.unwrap_err().kind(),
        std::io::ErrorKind::UnexpectedEof
    );
}

#[tokio::test]
async fn reverse_calls_are_bounded_and_cancellation_releases_the_slot() {
    use crate::context::{Outgoing, PluginContext};
    let (outgoing, mut frames) = tokio::sync::mpsc::channel(4);
    let context = PluginContext::new(outgoing, Duration::from_secs(3), 1).with_request_id(1);
    context.set_grants(&["echo".into()]);
    let _scope = context.request_scope();
    let first_context = context.clone();
    let first = tokio::spawn(async move {
        first_context
            .call_service("echo", "echo", serde_json::Value::Null)
            .await
    });
    assert!(matches!(frames.recv().await, Some(Outgoing::Frame(_))));
    assert_eq!(
        context
            .call_service("echo", "echo", serde_json::Value::Null)
            .await
            .unwrap_err()
            .code,
        -32002
    );
    assert!(frames.try_recv().is_err());
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    let Some(Outgoing::Frame(cancelled)) = frames.recv().await else {
        panic!("missing service cancellation");
    };
    assert_eq!(cancelled["method"], "notifications/cancelled");
    assert_eq!(cancelled["params"]["requestId"], 1);
    let next_context = context.clone();
    let next = tokio::spawn(async move {
        next_context
            .call_service("echo", "echo", serde_json::Value::Null)
            .await
    });
    let Some(Outgoing::Frame(frame)) = frames.recv().await else {
        panic!("missing service request");
    };
    context.reply(frame["id"].as_u64().unwrap(), Ok(serde_json::Value::Null));
    assert_eq!(next.await.unwrap().unwrap(), serde_json::Value::Null);
    context.close();
}

#[tokio::test]
async fn host_services_require_a_live_request_context_even_when_granted() {
    use crate::context::{Outgoing, PluginContext};
    let (outgoing, mut frames) = tokio::sync::mpsc::channel(4);
    let context = PluginContext::new(outgoing, Duration::from_secs(3), 1);
    context.set_grants(&["llm".into()]);
    assert!(context
        .call_service("llm", "complete", serde_json::Value::Null)
        .await
        .is_err());
    let request = context.clone().with_request_id(2);
    {
        let _scope = request.request_scope();
        let call = request.clone();
        let task = tokio::spawn(async move {
            call.call_service("llm", "complete", serde_json::Value::Null)
                .await
        });
        let Some(Outgoing::Frame(frame)) = frames.recv().await else {
            panic!("missing service request");
        };
        assert_eq!(frame["params"]["parentRequestId"], 2);
        request.reply(frame["id"].as_u64().unwrap(), Ok(serde_json::Value::Null));
        assert!(task.await.unwrap().is_ok());
    }
    assert!(request
        .call_service("llm", "complete", serde_json::Value::Null)
        .await
        .is_err());
    assert!(frames.try_recv().is_err());
}

#[tokio::test]
async fn a_host_service_timeout_sends_only_its_own_cancellation_and_releases_the_slot() {
    use crate::context::{Outgoing, PluginContext};
    let (outgoing, mut frames) = tokio::sync::mpsc::channel(4);
    let context = PluginContext::new(outgoing, Duration::from_millis(10), 1).with_request_id(1);
    context.set_grants(&["echo".into()]);
    let _scope = context.request_scope();
    assert_eq!(
        context
            .call_service("echo", "echo", serde_json::Value::Null)
            .await
            .unwrap_err()
            .code,
        -32001
    );
    let Some(Outgoing::Frame(first)) = frames.recv().await else {
        panic!("missing service request");
    };
    let Some(Outgoing::Frame(cancelled)) = frames.recv().await else {
        panic!("missing service cancellation");
    };
    assert_eq!(cancelled["method"], "notifications/cancelled");
    assert_eq!(cancelled["params"]["requestId"], first["id"]);
    context.reply(
        first["id"].as_u64().unwrap(),
        Ok(serde_json::Value::Bool(false)),
    );
    let next_context = context.clone();
    let next = tokio::spawn(async move {
        next_context
            .call_service("echo", "echo", serde_json::Value::Null)
            .await
    });
    let Some(Outgoing::Frame(frame)) = frames.recv().await else {
        panic!("missing next service request");
    };
    context.reply(
        frame["id"].as_u64().unwrap(),
        Ok(serde_json::Value::Bool(true)),
    );
    assert_eq!(next.await.unwrap().unwrap(), serde_json::Value::Bool(true));
    assert!(frames.try_recv().is_err());
}
