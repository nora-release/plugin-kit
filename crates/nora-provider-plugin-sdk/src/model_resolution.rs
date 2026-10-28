//! Private, per-request model selection before the host chooses an API encoder.
//! Models are complete `nora_ai::Model` JSON values so the SDK remains independent
//! of any particular inference library. Headers and selection IDs are transient.

use crate::CallContext;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveModelRequest {
    #[serde(flatten)]
    pub context: CallContext,
    pub model: Value,
    pub prompt: String,
    #[serde(default, rename = "sessionID")]
    pub session_id: Option<String>,
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    pub previous_failure: Option<PreviousFailure>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviousFailure {
    pub status: u16,
    pub body: String,
    #[serde(
        default,
        rename = "selectionID",
        skip_serializing_if = "Option::is_none"
    )]
    pub selection_id: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelResolution {
    pub model: Value,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(
        default,
        rename = "selectionID",
        skip_serializing_if = "Option::is_none"
    )]
    pub selection_id: Option<String>,
}

impl std::fmt::Debug for ModelResolution {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ModelResolution")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "../test/model_resolution.test.rs"]
mod tests;
