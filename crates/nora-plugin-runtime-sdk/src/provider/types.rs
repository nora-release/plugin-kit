use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use crate::protocol::RpcError;
use crate::streams::{StreamReader, StreamWriter};

/// What every question carries: the provider asked about, the account's
/// credentials as the plugin gave them (none while signing in), the user's
/// switches and the proxy to go through.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    pub provider: String,
    #[serde(default)]
    pub credentials: Option<Value>,
    #[serde(default)]
    pub options: BTreeMap<String, bool>,
    #[serde(default)]
    pub proxy: Option<String>,
}

impl Question {
    /// The account's credentials as `T`, the type the plugin keeps them in.
    pub fn credentials<T: serde::de::DeserializeOwned>(&self) -> Result<T, RpcError> {
        let credentials = self
            .credentials
            .clone()
            .ok_or_else(|| RpcError::invalid_params("The question carries no credentials"))?;
        serde_json::from_value(credentials)
            .map_err(|error| RpcError::invalid_params(format!("Unreadable credentials: {error}")))
    }

    /// Whether the user turned the switch named on.
    pub fn option(&self, key: &str) -> bool {
        self.options.get(key).copied().unwrap_or(false)
    }
}

/// `provider/signin`: start signing in the way named ("browser" or
/// "key"), with what the user filled in so far.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignIn {
    #[serde(flatten)]
    pub question: Question,
    pub way: String,
    #[serde(default)]
    pub inputs: BTreeMap<String, String>,
}

/// `provider/signin/finish`: whether a sign-in the user went to finish is
/// done; `code` is what they pasted back or answered, else empty.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignInFinish {
    #[serde(flatten)]
    pub question: Question,
    pub session: String,
    #[serde(default)]
    pub code: String,
}

/// Where a sign-in is.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// The user opens `url`, typing `code` there when there is one; the
    /// router asks `provider/signin/finish` with `session` until done.
    Open {
        url: String,
        code: Option<String>,
        session: String,
    },
    /// One more thing for the user to fill in, answered as the `code` of
    /// `provider/signin/finish`.
    Ask { session: String, ask: Ask },
    /// Signed in.
    Account(Account),
    /// The user isn't done yet.
    Waiting,
}

impl Serialize for Step {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        match self {
            Self::Open { url, code, session } => {
                map.serialize_entry("open", url)?;
                if let Some(code) = code {
                    map.serialize_entry("code", code)?;
                }
                map.serialize_entry("session", session)?;
            }
            Self::Ask { session, ask } => {
                map.serialize_entry("session", session)?;
                map.serialize_entry("ask", ask)?;
            }
            Self::Account(account) => map.serialize_entry("account", account)?,
            Self::Waiting => map.serialize_entry("waiting", &true)?,
        }
        map.end()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ask {
    pub key: String,
    pub label: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub secret: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
}

/// An account signed in. The router keeps `credentials` and hands them
/// back with every question about the account; the plugin keeps nothing.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    /// How the account shows, an email usually; unique per provider.
    pub name: String,
    pub credentials: Value,
    /// When the credentials expire (RFC 3339): the router asks
    /// `provider/refresh` shortly before.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

/// `provider/refresh`'s answer: the credentials renewed.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Renewed {
    pub credentials: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

/// A model an account can use. Only `id` is needed; the router fills in the
/// rest from models.dev.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<u64>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub reasoning: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub images: bool,
    /// The provider's APIs the model is served on, when not all of them:
    /// of its `api` and `apis` (`openai-chat`, `openai-responses`,
    /// `anthropic`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub apis: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<Cost>,
    /// The reasoning efforts the model takes, as the provider names them
    /// (`low`, `medium`, `high`, `xhigh`, `max`), weakest first; empty
    /// when the provider doesn't say. Giving any makes it a reasoning model.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub efforts: Vec<String>,
    /// The effort the provider uses when none is asked for, one of
    /// `efforts`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_effort: Option<String>,
    /// A line on what sets the model apart, such as a fast tier's price.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl Model {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            ..Self::default()
        }
    }
}

/// US dollars per million tokens.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cost {
    pub input: f64,
    pub output: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<f64>,
}

/// `provider/connect`'s answer: where the router sends the account's
/// requests itself, and the headers that sign them.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    /// Empty: the address the manifest declares.
    #[serde(rename = "baseURL", skip_serializing_if = "String::is_empty")]
    pub base_url: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
}

/// `provider/fetch`: one request the plugin sends itself. Its body is read
/// from `body`; the response's body is written to `response` after the
/// head is answered, from a task of the plugin's own.
pub struct Fetch {
    pub question: Question,
    pub method: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub body: StreamReader,
    pub response: StreamWriter,
}

/// `provider/fetch`'s answer: the response's status and headers, and the
/// account's credentials when sending rotated them.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchHead {
    pub status: u16,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

/// `provider/usage`'s answer: how much of its allowance an account used.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub windows: Vec<UsageWindow>,
    /// Resets the account holds, each starting its windows again at once.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets: Option<UsageResets>,
}

/// An allowance, or what is left of one. With `used` it is a share, shown
/// as a bar; with only `balance` it is an amount, shown as its number.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    pub name: String,
    /// The share used, 0–100.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub balance: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

/// The resets an account holds (ChatGPT's rate-limit resets): how many,
/// and each one, the soonest to run out first.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageResets {
    pub count: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cards: Vec<ResetCard>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetCard {
    /// The vendor's id for it, which spending this one names.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// When it runs out, RFC 3339; none when it never does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

/// `provider/checkin`'s answer: what a daily check-in came to.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Checkin {
    pub outcome: CheckinOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckinOutcome {
    Claimed,
    Done,
    Unavailable,
    Captcha,
    Failed,
}
