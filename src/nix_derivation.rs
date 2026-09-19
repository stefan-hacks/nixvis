//! On-demand dependency fetching via `nix derivation show`.
//!
//! When packages are loaded from search.nixos.org, they don't include dependency
//! graphs. This module spawns `nix derivation show nixpkgs#<attr> --json` to
//! fetch them lazily when a user opens the Deps or Graph tab.
//!
//! **Performance warning:** Each call evaluates the derivation in the Nix
//! evaluator. For a warm evaluator cache it's ~1–3 s; cold it can be 10–30 s.
//! We run it on a background thread and cache the result in the Index so the
//! same package is never fetched twice per session.

use std::collections::HashMap;
use std::process::{Command, Stdio};

/// Result of a single `nix derivation show` query.
#[derive(Debug, Clone)]
pub struct DerivationInfo {
    pub attr: String,
    pub inputs: Vec<String>,
    pub propagated: Vec<String>,
    pub native: Vec<String>,
}

/// Fetch derivation info for a single attribute path.
///
/// Returns `None` if Nix is not available, the attribute doesn't evaluate,
/// or JSON parsing fails.
pub fn fetch_derivation(attr: &str) -> Option<DerivationInfo> {
    // Normalize attr: if it doesn't contain '#', assume nixpkgs#prefix
    let installable = if attr.contains('#') {
        attr.to_string()
    } else {
        format!("nixpkgs#{attr}")
    };

    let output = Command::new("nix")
        .args([
            "derivation",
            "show",
            &installable,
            "--json",
            "--extra-experimental-features",
            "nix-command flakes",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let json_str = String::from_utf8(output.stdout).ok()?;
    parse_derivation_json(&json_str, attr)
}

/// Parse the JSON emitted by `nix derivation show`.
///
/// The top-level is a map from store path → derivation object. We only care
/// about the first (and usually only) entry.
fn parse_derivation_json(json: &str, attr: &str) -> Option<DerivationInfo> {
    #[derive(Debug, serde::Deserialize)]
    struct Derivation {
        #[serde(default)]
        input_builds: Vec<String>,
        #[serde(default)]
        input_drves: Vec<String>,
        #[serde(default)]
        input_sources: Vec<String>,
    }

    #[derive(Debug, serde::Deserialize)]
    struct Wrapper {
        #[serde(flatten)]
        entries: HashMap<String, Derivation>,
    }

    let wrapper: Wrapper = serde_json::from_str(json).ok()?;
    let (_store_path, drv) = wrapper.entries.into_iter().next()?;

    // inputBuilds are build-time dependencies (usually what users want).
    // inputDrvs are derivation dependencies (also useful).
    let inputs = drv
        .input_builds
        .iter()
        .chain(&drv.input_drves)
        .map(|s| extract_name(s))
        .collect();

    Some(DerivationInfo {
        attr: attr.to_string(),
        inputs,
        propagated: Vec::new(), // nix derivation show doesn't separate propagated
        native: drv.input_sources.iter().map(|s| extract_name(s)).collect(),
    })
}

/// Extract a human-readable name from a store path or drv path.
/// e.g. `/nix/store/…-glibc-2.39` → `glibc-2.39`
fn extract_name(path: &str) -> String {
    path.rsplit('/')
        .next()
        .unwrap_or(path)
        .split_once('-')
        .map(|(_, name)| name.to_string())
        .unwrap_or_else(|| path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_name() {
        assert_eq!(extract_name("/nix/store/abc123-glibc-2.39"), "glibc-2.39");
        assert_eq!(extract_name("glibc-2.39"), "2.39");
        assert_eq!(extract_name("hello"), "hello");
    }
}
