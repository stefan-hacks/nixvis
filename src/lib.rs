//! nixvis — interactive package explorer and dependency visualizer for GNU Nix.
//!
//! Indexes every GNU Nix package through an embedded Nix script executed by
//! `nix repl`, caches the result as gzipped JSON, and exposes it through a
//! keyboard-first terminal UI: fuzzy search, package details, dependency and
//! reverse-dependency trees, and a force-directed dependency graph.

pub mod app;
pub mod cache;
pub mod error;
pub mod graph;
pub mod index;
pub mod indexer;
pub mod model;
pub mod search;
pub mod theme;
pub mod ui;
#[cfg(feature = "web")]
pub mod web;

/// Canonical name and version, used for `--version` output and UI.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
