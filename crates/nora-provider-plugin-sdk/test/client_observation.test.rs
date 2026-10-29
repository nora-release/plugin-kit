use super::*;
use crate::{ClientObservation, ProviderPlugin};

#[derive(Default)]
struct Observer(std::sync::Mutex<Vec<ClientObservation>>);
#[async_trait::async_trait]
impl ProviderPlugin for Observer {
    fn supports_client_observation(&self) -> bool {
        true
    }
    async fn observe_client(&self, observation: ClientObservation) -> PluginResult<()> {
        self.0.lock().unwrap().push(observation);
        Ok(())
    }
}
struct OldPlugin;
#[async_trait::async_trait]
impl ProviderPlugin for OldPlugin {}

#[tokio::test]
async fn caller_observation_is_typed_bounded_and_feature_gated_without_account_or_http() {
    let observer = Observer::default();
    let value = json!({"userAgent":"Codex Desktop/0.160.0","version":"0.162.3","originator":"codex_desktop"});
    assert_eq!(
        dispatch(&observer, "observeClient", value.clone())
            .await
            .unwrap(),
        json!({"ok":true})
    );
    assert_eq!(
        serde_json::to_value(&observer.0.lock().unwrap()[0]).unwrap(),
        value
    );
    for invalid in [
        json!({"headers":{"authorization":"must-not-be-accepted"}}),
        json!({"credentials":{"access":"must-not-be-accepted"}}),
        json!({"version":123}),
        json!({"version":""}),
        json!({"userAgent":"x".repeat(8193)}),
        json!({"version":"x".repeat(257)}),
        json!({"originator":"x".repeat(1025)}),
        json!({"version":"0.162.3\nother"}),
    ] {
        assert!(dispatch(&observer, "observeClient", invalid).await.is_err());
    }
    assert_eq!(observer.0.lock().unwrap().len(), 1);
    assert_eq!(
        dispatch(&OldPlugin, "observeClient", json!({}))
            .await
            .unwrap_err()
            .code,
        "method_not_supported"
    );
}

#[tokio::test]
async fn caller_observation_negotiates_the_executable_capability_over_stdio() {
    use tokio::io::AsyncBufReadExt;
    let (client, server) = tokio::io::duplex(65536);
    let (input, output) = tokio::io::split(server);
    let (reader, mut writer) = tokio::io::split(client);
    let plugin = Arc::new(Observer::default());
    let task = tokio::spawn(serve_io(plugin.clone(), input, output));
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    writer.write_all(format!("{}\n",json!({"id":1,"method":"initialize","params":{"apiVersion":2,"pluginID":"fixture","pluginDirectory":"/plugin","dataDirectory":"/accounts","manifest":{}}})).as_bytes()).await.unwrap();
    reader.read_line(&mut line).await.unwrap();
    let value: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        value["result"]["capabilities"]["sdkFeatures"]["clientObservation"],
        1
    );
    writer.write_all(format!("{}\n",json!({"id":2,"method":"observeClient","params":{"version":"0.162.3","originator":"codex_desktop","subscriptionAccountsEnabled":true}})).as_bytes()).await.unwrap();
    line.clear();
    reader.read_line(&mut line).await.unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap(),
        json!({"id":2,"result":{"ok":true}})
    );
    assert_eq!(plugin.0.lock().unwrap().len(), 1);
    task.abort();
}
