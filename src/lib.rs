//! nixvis — interactive package explorer and dependency visualizer for the Nix ecosystem.
//!
//! Indexes Nix packages via `nix search --json` or loads an embedded JSON index,
//! caches the result as gzipped JSON, and exposes it through a keyboard-driven
//! terminal UI: fuzzy search, package details, dependency and reverse-dependency
//! trees, and a force-directed dependency graph.

pub mod app;
pub mod cache;
pub mod error;
pub mod graph;
pub mod index;
pub mod indexer;
pub mod model;
pub mod options;
pub mod search;
pub mod theme;
pub mod ui;
#[cfg(feature = "web")]
pub mod web;

/// Canonical name and version, used for `--version` output and UI.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
