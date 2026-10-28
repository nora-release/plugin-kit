use super::*;
use crate::{CallContext, ProviderPlugin};
#[derive(Default)]
struct Revoker(std::sync::Mutex<Vec<CallContext>>);
#[async_trait::async_trait]
impl ProviderPlugin for Revoker {
    fn supports_account_revocation(&self) -> bool {
        true
    }
    async fn revoke_account(&self, context: CallContext) -> PluginResult<()> {
        self.0.lock().unwrap().push(context);
        Ok(())
    }
}
struct OldPlugin;
#[async_trait::async_trait]
impl ProviderPlugin for OldPlugin {}
#[tokio::test]
async fn revocation_requires_support_and_passes_only_supplied_account_context() {
    let plugin = Revoker::default();
    dispatch(&plugin,"revokeAccount",json!({"account":"removed","credentials":{"refresh":"token"},"subscriptionAccountsEnabled":false})).await.unwrap();
    let seen = plugin.0.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].account, "removed");
    assert_eq!(seen[0].credentials.as_ref().unwrap()["refresh"], "token");
    assert!(!seen[0].subscription_accounts_enabled);
    assert_eq!(
        dispatch(&OldPlugin, "revokeAccount", json!({}))
            .await
            .unwrap_err()
            .code,
        "method_not_supported"
    );
}
#[tokio::test]
async fn revocation_capability_is_negotiated_from_the_executable() {
    use tokio::io::AsyncBufReadExt;
    let (client, server) = tokio::io::duplex(65536);
    let (input, output) = tokio::io::split(server);
    let (reader, mut writer) = tokio::io::split(client);
    let task = tokio::spawn(serve_io(Arc::new(Revoker::default()), input, output));
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    writer.write_all(format!("{}\n",json!({"id":1,"method":"initialize","params":{"apiVersion":1,"pluginID":"fixture","pluginDirectory":"/plugin","dataDirectory":"/accounts","manifest":{}}})).as_bytes()).await.unwrap();
    reader.read_line(&mut line).await.unwrap();
    let value: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        value["result"]["capabilities"]["sdkFeatures"]["accountRevocation"],
        1
    );
    task.abort();
}
