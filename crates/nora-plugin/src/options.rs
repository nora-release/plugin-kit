use nora_plugin_manifest::{Error, Result};
use std::collections::HashMap;

const SWITCHES: [&str; 2] = ["--verify-downloads", "--debug"];

/// `--name value` pairs and the few switches that take no value; each
/// option may be given once.
pub struct Options(HashMap<String, Option<String>>);

impl Options {
    pub fn parse(arguments: &[String]) -> Result<Self> {
        let mut options = HashMap::new();
        let mut arguments = arguments.iter();
        while let Some(key) = arguments.next() {
            if !key.starts_with("--") || options.contains_key(key) {
                return Err(Error::new(format!("Invalid or duplicate option: {key}")));
            }
            if SWITCHES.contains(&key.as_str()) {
                options.insert(key.clone(), None);
                continue;
            }
            match arguments.next() {
                Some(value) if !value.is_empty() && !value.starts_with("--") => {
                    options.insert(key.clone(), Some(value.clone()));
                }
                _ => return Err(Error::new(format!("Missing value for {key}"))),
            }
        }
        Ok(Self(options))
    }

    pub fn has(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    pub fn optional(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(Option::as_deref)
    }

    pub fn required(&self, key: &str) -> Result<&str> {
        self.optional(key)
            .ok_or_else(|| Error::new(format!("Missing {key}")))
    }
}
