use crate::{PluginError, PluginResult};
use serde::{Deserialize, Serialize};

/// Bounded public caller identity, carried privately without credentials.
/// Observers may update a local compatibility cache, never contact a vendor.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClientObservation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub originator: Option<String>,
}
impl ClientObservation {
    pub fn validate(&self) -> PluginResult<()> {
        for (value, max) in [
            (&self.user_agent, 8192),
            (&self.version, 256),
            (&self.originator, 1024),
        ] {
            if value.as_ref().is_some_and(|value| {
                value.is_empty() || value.len() > max || value.chars().any(char::is_control)
            }) {
                return Err(PluginError::invalid("The caller identity is invalid."));
            }
        }
        Ok(())
    }
    pub fn is_empty(&self) -> bool {
        self.user_agent.is_none() && self.version.is_none() && self.originator.is_none()
    }
}
