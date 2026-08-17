//! GUI host `/api` trust fence, static SPA files, plugin bundles, and `__DSH_BOOT__` injection.
//!
//! The axum listener lands in later tasks. This crate does not listen.

mod boot;
mod plugins;
mod static_files;
mod trust;

pub use boot::{WebBootEntry, WebBootGraph, inject_boot_manifest};
pub use plugins::{HostError, scan_client_packages, serve_plugin_js, serve_plugin_source_map};
pub use static_files::{StaticResponse, serve_spa};
pub use trust::{
    TrustError, assert_trusted_authority, is_loopback_hostname, is_privileged_method,
    is_trusted_api_request, privileged_requires_loopback,
};
