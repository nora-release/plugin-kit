//! `nora-plugin.json`, API version 2: one package, everything it adds to
//! Nora named under `contributes`.

use crate::error::{ensure, Error, Result};
use crate::text::is_semantic_version;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

pub const MANIFEST_FILE: &str = "nora-plugin.json";
pub const API_VERSION: u32 = 2;
/// Places in Nora where a panel can appear. A `provider` panel sits in each
/// provider configuration's settings and is told that configuration.
pub const PANEL_SLOTS: &[&str] = &["provider"];
/// Menus an action can join. A `provider` action is in each provider row's
/// menu and tells the page which configuration it was picked on.
pub const ACTION_MENUS: &[&str] = &["provider"];
/// What a panel or page may read from Nora. None of it is secret, since a
/// page cannot be kept from showing what it reads.
pub const READS: &[&str] = &["providers:list"];
const MAXIMUM_PANELS: usize = 8;
const MAXIMUM_ACTIONS: usize = 8;
const MAXIMUM_TITLE_BYTES: usize = 64;
const MAXIMUM_TITLES: usize = 16;
const SCRIPT_EXTENSIONS: &[&str] = &[
    "js", "mjs", "cjs", "ts", "mts", "py", "sh", "bash", "rb", "pl",
];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginManifest {
    pub api_version: u32,
    /// Lower-case letters, digits, `-`, `_` and `.`. A package that adds a
    /// provider serves its models under this ID.
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_host_version: Option<String>,
    /// The compiled executable Nora runs for the provider, hooks, middleware
    /// and masker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<Runtime>,
    pub contributes: Contributes,
    /// What the package's panels and page may read from Nora.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reads: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Runtime {
    /// A path inside the package to a compiled executable.
    pub entry: String,
}

/// Everything a package adds. The declarations Nora interprets itself
/// (hooks, middleware, masker) are checked by the host.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Contributes {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<Provider>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hooks: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub middleware: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub masker: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub panels: Vec<Panel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<Page>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Action>,
}

/// A model provider served under the package's ID.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Provider {
    /// The model API Nora speaks to the provider, such as `openai-responses`.
    pub api: String,
    #[serde(default, rename = "baseURL", skip_serializing_if = "String::is_empty")]
    pub base_url: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub auth_methods: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<Value>,
    /// Provider settings Nora interprets, such as account actions.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub config: Map<String, Value>,
    /// Sign-ins Nora can read from the provider's own apps on this computer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub local_sign_ins: Vec<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Panel {
    pub slot: String,
    pub title: String,
    /// The title by language code, for Nora's language.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub titles: BTreeMap<String, String>,
    /// An `.html` file inside the package.
    pub file: String,
}

/// The package's own page, with a place in Nora's navigation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Page {
    pub title: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub titles: BTreeMap<String, String>,
    pub file: String,
}

/// A menu item that opens the package's page and tells it what was picked.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Action {
    /// Short and lower-case, so the page can switch on it.
    pub id: String,
    pub menu: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub titles: BTreeMap<String, String>,
}

impl PluginManifest {
    /// Parses and checks a manifest's own structure. Files it names are
    /// checked by [`PluginManifest::validate_files`] once installed.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let manifest: Self = serde_json::from_slice(bytes)
            .map_err(|error| Error::new(format!("Invalid {MANIFEST_FILE}: {error}")))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        ensure(
            self.api_version == API_VERSION,
            "This plugin needs a different plugin API version",
        )?;
        validate_id(&self.id)?;
        ensure(
            plain(&self.name, 128)
                && plain(&self.version, 64)
                && is_semantic_version(&self.version),
            "A plugin needs a name and a strict semantic version",
        )?;
        for (field, value, maximum) in [
            ("publisher", &self.publisher, 128),
            ("summary", &self.summary, 2048),
        ] {
            if let Some(value) = value {
                ensure(
                    plain(value, maximum),
                    &format!("The plugin {field} must be short plain text"),
                )?;
            }
        }
        if let Some(minimum) = &self.minimum_host_version {
            ensure(
                is_semantic_version(minimum),
                "minimumHostVersion must be a semantic version",
            )?;
        }
        let contributes = &self.contributes;
        let runs = contributes.provider.is_some()
            || contributes.hooks.is_some()
            || contributes.middleware.is_some()
            || contributes.masker.is_some();
        let draws = !contributes.panels.is_empty() || contributes.page.is_some();
        ensure(runs || draws, "A plugin must contribute something to Nora")?;
        match &self.runtime {
            Some(runtime) => {
                ensure(
                    runs,
                    "Only a provider, hooks, middleware or a masker need a runtime",
                )?;
                validate_entry(&runtime.entry)?;
            }
            None => ensure(
                !runs,
                "A provider, hooks, middleware or a masker need a runtime entry",
            )?,
        }
        if let Some(provider) = &contributes.provider {
            ensure(
                plain(&provider.api, 64),
                "A provider needs the model API Nora speaks to it",
            )?;
            ensure(
                provider.models.len() <= 4096 && provider.auth_methods.len() <= 32,
                "A provider has too many models or sign-in methods",
            )?;
        }
        for (name, declaration) in [
            ("hooks", &contributes.hooks),
            ("middleware", &contributes.middleware),
            ("masker", &contributes.masker),
        ] {
            if let Some(declaration) = declaration {
                ensure(
                    declaration.is_object(),
                    &format!("contributes.{name} must be an object"),
                )?;
            }
        }
        self.validate_window()
    }

    fn validate_window(&self) -> Result<()> {
        let contributes = &self.contributes;
        ensure(
            contributes.panels.len() <= MAXIMUM_PANELS,
            &format!("A plugin can add at most {MAXIMUM_PANELS} panels"),
        )?;
        for panel in &contributes.panels {
            ensure(
                PANEL_SLOTS.contains(&panel.slot.as_str()),
                &format!("Unknown panel slot: {}", panel.slot),
            )?;
            validate_titles(&panel.title, &panel.titles)?;
            validate_html(&panel.file)?;
        }
        if let Some(page) = &contributes.page {
            validate_titles(&page.title, &page.titles)?;
            validate_html(&page.file)?;
        }
        if !contributes.actions.is_empty() {
            ensure(
                contributes.page.is_some(),
                "Actions open the plugin's page, so they need one",
            )?;
            ensure(
                contributes.actions.len() <= MAXIMUM_ACTIONS,
                &format!("A plugin can add at most {MAXIMUM_ACTIONS} actions"),
            )?;
        }
        let mut ids = BTreeSet::new();
        for action in &contributes.actions {
            ensure(
                action_id(&action.id) && ids.insert(action.id.as_str()),
                &format!(
                    "Action IDs must be short, lower-case and unique: {}",
                    action.id
                ),
            )?;
            ensure(
                ACTION_MENUS.contains(&action.menu.as_str()),
                &format!("Unknown action menu: {}", action.menu),
            )?;
            validate_titles(&action.title, &action.titles)?;
        }
        let reads: BTreeSet<_> = self.reads.iter().collect();
        ensure(
            reads.len() == self.reads.len()
                && self.reads.iter().all(|read| READS.contains(&read.as_str())),
            "Plugin reads are unknown or repeated",
        )?;
        ensure(
            self.reads.is_empty() || draws(contributes),
            "Only a plugin with panels or a page reads from Nora",
        )
    }

    /// Every file the manifest names must be a regular file inside the
    /// installed package, after resolving symbolic links.
    pub fn validate_files(&self, directory: &Path) -> Result<()> {
        let root = directory
            .canonicalize()
            .map_err(|error| Error::new(format!("{}: {error}", directory.display())))?;
        let files = self
            .runtime
            .iter()
            .map(|runtime| &runtime.entry)
            .chain(self.contributes.panels.iter().map(|panel| &panel.file))
            .chain(self.contributes.page.iter().map(|page| &page.file));
        for file in files {
            let inside = directory
                .join(file)
                .canonicalize()
                .is_ok_and(|real| real.starts_with(&root) && real.is_file());
            ensure(inside, &format!("{file} must be a file inside the plugin"))?;
        }
        Ok(())
    }
}

fn draws(contributes: &Contributes) -> bool {
    !contributes.panels.is_empty() || contributes.page.is_some()
}

/// Lower-case letters, digits, `-`, `_` and `.`, starting with a letter or
/// digit, at most 64 bytes.
pub fn validate_id(id: &str) -> Result<()> {
    let valid = !id.is_empty()
        && id.len() <= 64
        && id.starts_with(|first: char| first.is_ascii_lowercase() || first.is_ascii_digit())
        && id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        });
    ensure(
        valid,
        "A plugin ID must use 1–64 lower-case letters, digits, hyphens, underscores or dots",
    )
}

fn action_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 40
        && id.starts_with(|first: char| first.is_ascii_lowercase() || first.is_ascii_digit())
        && id.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

fn validate_titles(title: &str, titles: &BTreeMap<String, String>) -> Result<()> {
    ensure(
        plain(title, MAXIMUM_TITLE_BYTES)
            && titles.len() <= MAXIMUM_TITLES
            && titles.iter().all(|(language, title)| {
                !language.is_empty() && language.len() <= 16 && plain(title, MAXIMUM_TITLE_BYTES)
            }),
        "Page, panel and action titles must be short plain text",
    )
}

fn relative(path: &str) -> bool {
    !path.is_empty()
        && !Path::new(path).is_absolute()
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

fn validate_html(file: &str) -> Result<()> {
    ensure(
        relative(file)
            && Path::new(file)
                .extension()
                .and_then(|extension| extension.to_str())
                == Some("html"),
        "Page and panel files must be .html files inside the plugin",
    )
}

fn validate_entry(entry: &str) -> Result<()> {
    ensure(
        relative(entry),
        "The runtime entry must be a path inside the plugin",
    )?;
    let script = Path::new(entry)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| SCRIPT_EXTENSIONS.contains(&extension));
    ensure(!script, "The runtime entry must be a compiled executable")
}

fn plain(text: &str, maximum: usize) -> bool {
    !text.trim().is_empty() && text.len() <= maximum && !text.chars().any(char::is_control)
}
