//! NixOS and Home-Manager options support.
//!
//! Provides data types and search for `nixos-option` and `home-manager-option`
//! style configuration entries. Options are stored in a separate JSON file
//! that is loaded at startup (after the main package index is ready).


use serde::{Deserialize, Serialize};

/// A single NixOS or Home-Manager configuration option.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NixosOption {
    pub name: String,
    pub description: String,
    pub option_type: String,
    pub default: String,
    pub example: String,
    #[serde(alias = "source")]
    pub declared_in: String,
}

/// A single Home-Manager configuration option.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HmOption {
    pub name: String,
    pub description: String,
    pub option_type: String,
    pub default: String,
    pub example: String,
    #[serde(alias = "source")]
    pub declared_in: String,
}

/// Collection of NixOS and Home-Manager options loaded from JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionsDoc {
    pub nixos_options: Vec<NixosOption>,
    pub hm_options: Vec<HmOption>,
}

/// An in-memory searchable index over options.
pub struct OptionsIndex {
    pub doc: OptionsDoc,
}

impl OptionsIndex {
    /// Load options from embedded JSON files.
    pub fn load() -> Result<Self, crate::error::IndexerError> {
        let nixos_raw = include_str!("../data/nixos-options.json");
        let hm_raw = include_str!("../data/hm-options.json");

        let nixos_options: Vec<NixosOption> = serde_json::from_str(nixos_raw).unwrap_or_default();
        let hm_options: Vec<HmOption> = serde_json::from_str(hm_raw).unwrap_or_default();

        Ok(OptionsIndex {
            doc: OptionsDoc {
                nixos_options,
                hm_options,
            },
        })
    }

    /// Search NixOS options by name or description (case-insensitive substring).
    pub fn search_nixos(&self, query: &str) -> Vec<&NixosOption> {
        let q = query.to_lowercase();
        self.doc
            .nixos_options
            .iter()
            .filter(|opt| {
                opt.name.to_lowercase().contains(&q) || opt.description.to_lowercase().contains(&q)
            })
            .collect()
    }

    /// Search Home-Manager options by name or description.
    pub fn search_hm(&self, query: &str) -> Vec<&HmOption> {
        let q = query.to_lowercase();
        self.doc
            .hm_options
            .iter()
            .filter(|opt| {
                opt.name.to_lowercase().contains(&q) || opt.description.to_lowercase().contains(&q)
            })
            .collect()
    }

    pub fn nixos_count(&self) -> usize {
        self.doc.nixos_options.len()
    }

    pub fn hm_count(&self) -> usize {
        self.doc.hm_options.len()
    }
}
