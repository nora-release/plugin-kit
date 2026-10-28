use crate::error::{ensure, Result};
use regex::Regex;
use serde_json::{Map, Value};
use std::sync::LazyLock;

static IDENTIFIER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[A-Za-z0-9][A-Za-z0-9-]*(?:\.[A-Za-z0-9][A-Za-z0-9-]*)+$").unwrap()
});
static SEMANTIC_VERSION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$",
    )
    .unwrap()
});

pub fn is_semantic_version(value: &str) -> bool {
    SEMANTIC_VERSION.is_match(value)
}

pub(crate) fn object<'a>(value: &'a Value, field: &str) -> Result<&'a Map<String, Value>> {
    value
        .as_object()
        .ok_or_else(|| crate::Error::new(format!("{field} must be a JSON object")))
}

/// Nonempty plain text of at most `maximum` UTF-8 bytes.
pub(crate) fn text<'a>(value: Option<&'a Value>, field: &str, maximum: usize) -> Result<&'a str> {
    let text = value.and_then(Value::as_str).unwrap_or_default();
    ensure(
        !text.trim().is_empty() && text.len() <= maximum && !text.chars().any(char::is_control),
        &format!("{field} must be nonempty plain text of at most {maximum} bytes"),
    )?;
    Ok(text)
}

pub(crate) fn identifier<'a>(value: Option<&'a Value>, field: &str) -> Result<&'a str> {
    let value = text(value, field, 255)?;
    ensure(
        IDENTIFIER.is_match(value),
        &format!("{field} must be a dot-separated identifier"),
    )?;
    Ok(value)
}

pub(crate) fn array<'a>(
    value: Option<&'a Value>,
    field: &str,
    maximum: usize,
) -> Result<&'a [Value]> {
    match value.and_then(Value::as_array) {
        Some(items) if items.len() <= maximum => Ok(items),
        _ => Err(crate::Error::new(format!(
            "{field} exceeds its limit or is not an array"
        ))),
    }
}

/// A `/`-separated path inside the plugin with no empty, `.`, or `..` part.
pub(crate) fn relative_resource_path<'a>(value: Option<&'a Value>, field: &str) -> Result<&'a str> {
    let path = value.and_then(Value::as_str);
    let valid = path.is_some_and(|path| {
        path.len() <= 1024
            && !path
                .chars()
                .any(|character| character == '\\' || character.is_control())
            && path
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != "..")
    });
    ensure(
        valid,
        &format!("{field} must be a relative resource path inside the plugin"),
    )?;
    Ok(path.unwrap_or_default())
}

/// JSON's number 1, written as `1` or `1.0`.
pub(crate) fn is_one(value: Option<&Value>) -> bool {
    value.and_then(Value::as_f64) == Some(1.0)
}
