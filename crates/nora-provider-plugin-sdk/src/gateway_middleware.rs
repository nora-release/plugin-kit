//! Versioned synchronous gateway processing, independent of gatewayHooks.
//! Implementations using an older SDK can provide this protocol through
//! ProviderPlugin::call without changing their authentication/HTTP interface.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

pub const VERSION: u64 = 1;
pub const METHOD: &str = "gatewayMiddleware";
pub const MAX_BODY: usize = 32 * 1024 * 1024;
pub const MAX_OPTIONS: usize = 32 * 1024;
pub const MAX_STATE: usize = 64 * 1024;
pub const REQUEST_LIMIT: Duration = Duration::from_millis(250);
pub const EVENT_LIMIT: Duration = Duration::from_millis(50);
pub const ORDER_LIMIT: Duration = Duration::from_millis(20);

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Declaration {
    pub version: u64,
    pub request: bool,
    pub provider: bool,
    pub event: bool,
    pub response: bool,
    pub events: Vec<String>,
    pub strategies: Vec<String>,
    /// The extension's options until the user sets their own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<serde_json::Value>,
}
impl Declaration {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != VERSION
            || (!self.request
                && !self.provider
                && !self.event
                && !self.response
                && self.strategies.is_empty())
            || self.events.len() > 128
            || self.events.iter().any(|s| s.is_empty() || s.len() > 256)
            || self.strategies.len() > 64
            || self.strategies.iter().any(|s| !valid_name(s))
            || self
                .strategies
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.strategies.len()
        {
            return Err("The gateway middleware declaration is invalid".into());
        }
        Ok(())
    }
    pub fn participates(&self, phase: Phase, event: Option<&str>) -> bool {
        match phase {
            Phase::Request => self.request,
            Phase::Provider => self.provider,
            Phase::Event => {
                self.event
                    && (self.events.is_empty()
                        || event.is_some_and(|e| self.events.iter().any(|v| v == e)))
            }
            Phase::Response => self.response,
        }
    }
}
pub fn valid_name(name: &str) -> bool {
    name.len() <= 128
        && name
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphanumeric())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
pub fn strategy_id(name: &str) -> Option<(&str, &str)> {
    let (plugin, strategy) = name.split_once('/')?;
    (valid_name(plugin) && valid_name(strategy)).then_some((plugin, strategy))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Request,
    Provider,
    Event,
    Response,
}
impl Phase {
    pub fn name(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Provider => "provider",
            Self::Event => "event",
            Self::Response => "response",
        }
    }
    pub fn limit(self) -> Duration {
        if self == Self::Event {
            EVENT_LIMIT
        } else {
            REQUEST_LIMIT
        }
    }
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Context {
    pub protocol: String,
    pub model: String,
    pub path: String,
    pub agent: String,
    pub stream: bool,
    pub status: u16,
    pub provider: Value,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Rejection {
    pub status: u16,
    pub message: String,
}
#[derive(Debug, PartialEq)]
pub enum Action {
    Keep,
    Replace(Value),
    Drop,
    Reject(Rejection),
}
pub struct Reply {
    pub action: Action,
    pub state: Value,
}
pub fn reply(phase: Phase, result: Value) -> Result<Reply, String> {
    if result["version"] != VERSION {
        return Err("The middleware result version is unsupported".into());
    }
    let state = result.get("state").cloned().unwrap_or_else(|| json!({}));
    if !state.is_object() || !crate::gateway_hooks::fits(&state, MAX_STATE) {
        return Err("The middleware state is invalid".into());
    }
    let action = match result["action"].as_str() {
        Some("keep") => Action::Keep,
        Some("replace") => {
            let value = result
                .get("body")
                .ok_or("The middleware result is missing its body")?;
            if !value.is_object() && !value.is_array()
                || !crate::gateway_hooks::fits(value, MAX_BODY)
            {
                return Err("The middleware body is invalid".into());
            }
            Action::Replace(value.clone())
        }
        Some("drop") if phase == Phase::Event => Action::Drop,
        Some("reject") if phase == Phase::Request => {
            let status = result["reject"]["status"]
                .as_u64()
                .filter(|s| (400..=599).contains(s))
                .unwrap_or(403) as u16;
            let message = result["reject"]["message"]
                .as_str()
                .unwrap_or("turned away by a gateway middleware");
            if message.len() > 16 * 1024 {
                return Err("The middleware rejection is too large".into());
            }
            Action::Reject(Rejection {
                status,
                message: message.into(),
            })
        }
        Some("drop" | "reject") => Action::Keep,
        _ => return Err("The middleware action is invalid".into()),
    };
    Ok(Reply { action, state })
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Seat {
    pub key: String,
    pub account: bool,
    pub known: bool,
    pub learns: bool,
    pub used: f64,
    pub soon: Vec<u64>,
    pub pace: f64,
    pub weight: u32,
    pub resting: bool,
    pub tokens: f64,
}
impl Default for Seat {
    fn default() -> Self {
        Self {
            key: String::new(),
            account: false,
            known: false,
            learns: false,
            used: 0.,
            soon: vec![],
            pace: 0.,
            weight: 1,
            resting: false,
            tokens: 0.,
        }
    }
}
pub fn order(result: &Value, seats: usize) -> Option<Vec<usize>> {
    if result["version"] != VERSION {
        return None;
    }
    let values = result["order"].as_array()?;
    if values.len() != seats {
        return None;
    }
    let mut seen = std::collections::BTreeSet::new();
    values
        .iter()
        .map(|v| {
            let n = usize::try_from(v.as_u64()?).ok()?;
            (n < seats && seen.insert(n)).then_some(n)
        })
        .collect()
}

#[cfg(test)]
#[path = "../test/gateway_middleware.test.rs"]
mod tests;
