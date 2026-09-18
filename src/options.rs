//! NixOS and Home-Manager options support.
//!
//! Provides data types and search for `nixos-option` and `home-manager-option`
//! style configuration entries. Options are stored in a separate JSON file
//! (`data/nixos-options.json` and `data/hm-options.json`) and searched
//! independently of the package index.
//!
//! # Future work
//! - Live indexing via `nixos-option --json` or `manix`.
//! - Integration into the TUI as a separate tab or view mode.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// A NixOS configuration option (e.g. `services.openssh.enable`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NixosOption {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub option_type: String,
    #[serde(default)]
    pub default: String,
    #[serde(default)]
    pub example: String,
    #[serde(default)]
    pub source: String,
}

/// A Home-Manager configuration option (e.g. `programs.git.enable`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HmOption {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub option_type: String,
    #[serde(default)]
    pub default: String,
    #[serde(default)]
    pub example: String,
    #[serde(default)]
    pub source: String,
}

/// A collection of options loaded from JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionsDoc {
    pub nixos_options: Vec<NixosOption>,
    pub hm_options: Vec<HmOption>,
}

/// Simple in-memory options index with fuzzy search.
pub struct OptionsIndex {
    pub nixos: Vec<Arc<NixosOption>>,
    pub hm: Vec<Arc<HmOption>>,
}

impl OptionsDoc {
    pub fn empty() -> Self {
        Self {
            nixos_options: Vec::new(),
            hm_options: Vec::new(),
        }
    }
}

impl OptionsIndex {
    pub fn from_doc(doc: OptionsDoc) -> Self {
        Self {
            nixos: doc.nixos_options.into_iter().map(Arc::new).collect(),
            hm: doc.hm_options.into_iter().map(Arc::new).collect(),
        }
    }

    pub fn search_nixos(
        &self,
        query: &str,
        limit: usize,
    ) -> Vec<(Arc<NixosOption>, f64)> {
        let mut scored: Vec<(Arc<NixosOption>, f64)> = self
            .nixos
            .iter()
            .map(|opt| {
                let hay = format!("{} {}", opt.name, opt.description);
                let score = fuzzy_score(query, &hay);
                (Arc::clone(opt), score)
            })
            .filter(|(_, s)| *s > 0.0)
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        scored.truncate(limit);
        scored
    }

    pub fn search_hm(
        &self,
        query: &str,
        limit: usize,
    ) -> Vec<(Arc<HmOption>, f64)> {
        let mut scored: Vec<(Arc<HmOption>, f64)> = self
            .hm
            .iter()
            .map(|opt| {
                let hay = format!("{} {}", opt.name, opt.description);
                let score = fuzzy_score(query, &hay);
                (Arc::clone(opt), score)
            })
            .filter(|(_, s)| *s > 0.0)
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        scored.truncate(limit);
        scored
    }
}

/// Simple fuzzy scorer: counts matching characters in order.
fn fuzzy_score(needle: &str, hay: &str) -> f64 {
    let needle = needle.to_lowercase();
    let hay = hay.to_lowercase();
    if needle.is_empty() {
        return 1.0;
    }
    if hay.contains(&needle) {
        return 2.0 + (needle.len() as f64 / hay.len() as f64);
    }
    let mut ni = needle.chars();
    let mut count = 0;
    let mut matched = 0;
    if let Some(mut nc) = ni.next() {
        for hc in hay.chars() {
            if hc == nc {
                matched += 1;
                if let Some(next) = ni.next() {
                    nc = next;
                } else {
                    break;
                }
            }
            count += 1;
            if count > 1000 {
                break;
            }
        }
    }
    if matched == needle.len() {
        1.0 + (matched as f64 / count.max(1) as f64)
    } else {
        0.0
    }
}

/// Embed sample NixOS options JSON.
pub const SAMPLE_NIXOS_OPTIONS: &str = include_str!("../data/nixos-options.json");

/// Embed sample Home-Manager options JSON.
pub const SAMPLE_HM_OPTIONS: &str = include_str!("../data/hm-options.json");
