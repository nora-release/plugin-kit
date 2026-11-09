use super::*;
use crate::protocol::{DEFAULT_STREAM_WINDOW, MIN_STREAM_WINDOW};
use serde_json::json;
use std::time::Duration;

/// A host table and a plugin table, each one's frames delivered to the
/// other by a pump task. Violations are counted, not delivered further.
struct Pair {
    host: Arc<StreamTable>,
    plugin: Arc<StreamTable>,
    violations: Arc<Mutex<Vec<String>>>,
}

fn pair(window: u64) -> Pair {
    let (host_out, host_frames) = mpsc::channel::<Value>(1024);
    let (plugin_out, plugin_frames) = mpsc::channel::<Value>(1024);
    let host = StreamTable::new(Side::Host, window, Arc::new(host_out));
    let plugin = StreamTable::new(Side::Plugin, window, Arc::new(plugin_out));
    host.set_write_window(Some(plugin.read_window())).unwrap();
    plugin.set_write_window(Some(host.read_window())).unwrap();
    let violations = Arc::new(Mutex::new(Vec::new()));
    pump(host_frames, Arc::clone(&plugin), Arc::clone(&violations));
    pump(plugin_frames, Arc::clone(&host), Arc::clone(&violations));
    Pair {
        host,
        plugin,
        violations,
    }
}

fn pump(
    mut frames: mpsc::Receiver<Value>,
    to: Arc<StreamTable>,
    violations: Arc<Mutex<Vec<String>>>,
) {
    tokio::spawn(async move {
        while let Some(frame) = frames.recv().await {
            let params = frame["params"].clone();
            let result = match frame["method"].as_str() {
                Some(STREAM_DATA) => to.on_data(params),
                Some(STREAM_CREDIT) => to.on_credit(params),
                other => panic!("unexpected frame {other:?}"),
            };
            if let Err(violation) = result {
                violations.lock().unwrap().push(violation.0);
            }
        }
    });
}

const CALL: CallKey = CallKey { local: true, id: 7 };
const ACCEPTED: CallKey = CallKey {
    local: false,
    id: 7,
};

#[tokio::test]
async fn a_stream_larger_than_the_window_arrives_whole_and_in_order() {
    let pair = pair(MIN_STREAM_WINDOW);
    let writer = pair.host.open_writer(CALL).unwrap();
    let (reader, _) = pair
        .plugin
        .accept(ACCEPTED, Some(writer.id()), None)
        .unwrap();
    let mut reader = reader.unwrap();
    let sent: Vec<u8> = (0..1_000_000u32).map(|i| (i % 251) as u8).collect();
    let expected = sent.clone();
    let writing = tokio::spawn(async move {
        let mut writer = writer;
        writer.write(&sent).await.unwrap();
        writer.finish().await.unwrap();
    });
    let received = reader.read_to_end(2_000_000).await.unwrap();
    writing.await.unwrap();
    assert_eq!(received, expected);
    assert!(pair.violations.lock().unwrap().is_empty());
    assert!(!pair.host.has_open_streams(CALL));
}

#[tokio::test]
async fn a_writer_waits_while_its_reader_takes_nothing() {
    let pair = pair(MIN_STREAM_WINDOW);
    let mut writer = pair.host.open_writer(CALL).unwrap();
    let (reader, _) = pair
        .plugin
        .accept(ACCEPTED, Some(writer.id()), None)
        .unwrap();
    let mut reader = reader.unwrap();
    let big = vec![1u8; usize::try_from(MIN_STREAM_WINDOW * 3).unwrap()];
    let blocked = tokio::time::timeout(Duration::from_millis(200), writer.write(&big)).await;
    assert!(blocked.is_err(), "the writer went past the reader's window");
    // reading frees credit, and the next write goes through
    let mut taken = 0;
    while taken < MIN_STREAM_WINDOW / 2 {
        taken += reader.read().await.unwrap().unwrap().len() as u64;
    }
    assert!(pair.violations.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_cancelled_call_ends_its_streams_on_both_sides() {
    let pair = pair(DEFAULT_STREAM_WINDOW);
    let reader = pair.host.open_reader(CALL).unwrap();
    let (_, writer) = pair
        .plugin
        .accept(ACCEPTED, None, Some(reader.id()))
        .unwrap();
    let mut writer = writer.unwrap();
    let mut reader = reader;
    writer.write(b"head").await.unwrap();
    assert_eq!(reader.read().await.unwrap().unwrap(), b"head");
    let cancelled = RpcError::new(-32800, "cancelled");
    pair.host.close_call(CALL, &cancelled);
    pair.plugin.close_call(ACCEPTED, &cancelled);
    assert_eq!(reader.read().await.unwrap().unwrap_err(), cancelled);
    assert!(reader.read().await.is_none());
    assert_eq!(writer.write(b"late").await.unwrap_err(), cancelled);
    tokio::time::sleep(Duration::from_millis(50)).await;
    // a frame of the closed stream that was already on the wire is ignored
    assert!(pair
        .host
        .on_data(json!({"stream": reader.id(), "data": "eA=="}))
        .is_ok());
    assert!(pair.violations.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_writer_that_gives_up_or_is_dropped_tells_its_reader() {
    let pair = pair(DEFAULT_STREAM_WINDOW);
    let reader = pair.host.open_reader(CALL).unwrap();
    let (_, writer) = pair
        .plugin
        .accept(ACCEPTED, None, Some(reader.id()))
        .unwrap();
    let mut reader = reader;
    writer
        .unwrap()
        .abort(RpcError::new(-32050, "upstream failed"))
        .await
        .unwrap();
    assert_eq!(reader.read().await.unwrap().unwrap_err().code, -32050);

    let other = CallKey { local: true, id: 8 };
    let reader = pair.host.open_reader(other).unwrap();
    let (_, writer) = pair
        .plugin
        .accept(
            CallKey {
                local: false,
                id: 8,
            },
            None,
            Some(reader.id()),
        )
        .unwrap();
    let mut reader = reader;
    drop(writer);
    assert_eq!(reader.read().await.unwrap().unwrap_err().code, -32000);
}

#[tokio::test]
async fn stream_ids_are_the_callers_and_never_reused() {
    let pair = pair(DEFAULT_STREAM_WINDOW);
    let first = pair.host.open_writer(CALL).unwrap();
    assert_eq!(first.id() % 2, 1);
    // the plugin's own parity, a reuse, or one id for both streams
    assert!(pair.plugin.accept(ACCEPTED, Some(2), None).is_err());
    assert!(pair.plugin.accept(ACCEPTED, Some(first.id()), None).is_ok());
    assert!(pair
        .plugin
        .accept(ACCEPTED, Some(first.id()), None)
        .is_err());
    assert!(pair.plugin.accept(ACCEPTED, Some(9), Some(9)).is_err());
    let reader = pair.plugin.open_reader(CALL).unwrap();
    assert_eq!(reader.id(), 2);
}

#[tokio::test]
async fn frames_a_peer_should_not_send_are_violations() {
    let pair = pair(MIN_STREAM_WINDOW);
    let reader = pair.host.open_reader(CALL).unwrap();
    let writer = pair.host.open_writer(CALL).unwrap();
    // never allocated
    assert!(pair
        .host
        .on_data(json!({"stream": 99, "data": ""}))
        .is_err());
    assert!(pair
        .host
        .on_credit(json!({"stream": 99, "bytes": 1}))
        .is_err());
    // data from the stream's own reader, credit from its writer
    assert!(pair
        .host
        .on_data(json!({"stream": writer.id(), "data": ""}))
        .is_err());
    assert!(pair
        .host
        .on_credit(json!({"stream": reader.id(), "bytes": 1}))
        .is_err());
    // an error that doesn't end the stream, data that isn't base64
    assert!(pair
        .host
        .on_data(json!({"stream": reader.id(), "error": {"code": 1, "message": "x"}}))
        .is_err());
    assert!(pair
        .host
        .on_data(json!({"stream": reader.id(), "data": "!"}))
        .is_err());
    // more than the window granted
    let over =
        base64::engine::general_purpose::STANDARD.encode(vec![
            0u8;
            usize::try_from(MIN_STREAM_WINDOW)
                .unwrap()
        ]);
    assert!(pair
        .host
        .on_data(json!({"stream": reader.id(), "data": over}))
        .is_err());
}

#[tokio::test]
async fn a_closed_connection_ends_every_stream() {
    let pair = pair(DEFAULT_STREAM_WINDOW);
    let mut reader = pair.host.open_reader(CALL).unwrap();
    let mut writer = pair.host.open_writer(CALL).unwrap();
    pair.host.close_all(RpcError::new(-32000, "gone"));
    assert_eq!(reader.read().await.unwrap().unwrap_err().message, "gone");
    assert_eq!(writer.write(b"x").await.unwrap_err().message, "gone");
    assert!(pair.host.open_reader(CALL).is_err());
}

#[tokio::test]
async fn a_writer_with_less_credit_than_a_chunk_writes_what_fits() {
    // a short frame leaves the reader holding back less than half its
    // window, and the writer less than a whole chunk: it must still go on
    let pair = pair(MIN_STREAM_WINDOW);
    let writer = pair.host.open_writer(CALL).unwrap();
    let (reader, _) = pair
        .plugin
        .accept(ACCEPTED, Some(writer.id()), None)
        .unwrap();
    let mut reader = reader.unwrap();
    let writing = tokio::spawn(async move {
        let mut writer = writer;
        for size in [4096usize, 300_000, 17, 200_000] {
            writer.write(&vec![7u8; size]).await.unwrap();
        }
        writer.finish().await.unwrap();
    });
    let all = tokio::time::timeout(Duration::from_secs(5), reader.read_to_end(1_000_000))
        .await
        .expect("the writer and reader waited on each other")
        .unwrap();
    writing.await.unwrap();
    assert_eq!(all.len(), 4096 + 300_000 + 17 + 200_000);
    assert!(pair.violations.lock().unwrap().is_empty());
}
