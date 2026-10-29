//! Nora plugin manifests, marketplace registries, and release archives.
//!
//! This crate is the one validator shared by the `nora-plugin` publishing
//! tool and by Nora's plugin hosts, so a package that passes here is the
//! package Nora accepts.

mod archive;
mod error;
mod icon;
mod native;
pub mod package;
mod registry;
mod text;

pub use archive::{manifest_from_archive, sha256_hex, verify_archive, MAXIMUM_ARCHIVE_BYTES};
pub use error::{Error, Result};
pub use icon::{validate_icon_data, MAXIMUM_ICON_BYTES};
pub use native::{validate_native_manifest, MAXIMUM_MANIFEST_BYTES};
pub use package::{Action, Contributes, Page, Panel, PluginManifest, Provider, Runtime};
pub use registry::{
    canonical, encode_uri_component, format_registry, repository_path, validate_entry,
    validate_registry, MAXIMUM_REGISTRY_PLUGINS,
};
pub use text::is_semantic_version;
