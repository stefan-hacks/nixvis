//! Fast online package search via search.nixos.org Elasticsearch backend.
//!
//! This is how `nh` (nix-community/nh) achieves sub-second fuzzy search:
//! instead of running `nix search --json` locally (which evaluates nixpkgs
//! and takes 30–120 s), it POSTS to the public Elasticsearch endpoint
//! `https://search.nixos.org/backend/latest-<VERSION>-<CHANNEL>/_search`
//! with a multi_match query over indexed package fields.
//!
//! The API is documented indirectly at <https://github.com/NixOS/nixos-search>
//! and the hard-coded credentials are public knowledge (used by the
//! nixos.org frontend and nh alike).
//!
//! Advantages over `nix search --json`:
//! - Response time: ~1–50 ms for 20 results.
//! - No local Nix evaluation needed.
//! - Works on machines without the nixpkgs flake checked out.
//!
//! Limitations:
//! - Requires internet connectivity.
//! - Only returns packages that are indexed (nixos.org tracks nixos-unstable
//!   and latest stable release).
//! - No dependency graph data (just dep_count). We fall back to embedded JSON
//!   for the dependency visualiser.

use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use crate::model::{Header, IndexDoc, PkgJson};

/// Backend index version. Must match the version the search.nixos.org frontend
/// expects. Incremented when the mapping changes. nh bundles this in
/// `BACKEND_VERSION`; we hard-code the same value.
const BACKEND_VERSION: u32 = 51;

/// Hard-coded read-only credentials exposed by the upstream frontend.
/// See <https://github.com/NixOS/nixos-search/blob/744ec58e082a3fcdd741b2c9b0654a0f7fda4603/frontend/src/index.js>
const USER: &str = "aWVSALXpZv";
const PASS: &str = "X8gPHnzL52wFEekuxsfQ9cSh";

/// Default channel to query. `nixos-unstable` has the most up-to-date packages.
const DEFAULT_CHANNEL: &str = "nixos-unstable";

/// HTTP timeout for the Elasticsearch request.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Search nixpkgs packages via search.nixos.org and return an `IndexDoc`.
///
/// `query`:   the user's search string (e.g. `"hello"` or `""` for all).
/// `limit`:   max results to fetch (the API paginates; 20 is a good default).
/// `channel`: e.g. `"nixos-unstable"` or `"nixos-24.05"`.
///
/// On failure (network, parse, missing index) returns `None` so the caller
/// can fall back to the embedded JSON cache.
pub fn search_packages(query: &str, limit: usize, channel: &str) -> Option<IndexDoc> {
    let client = Client::builder().timeout(REQUEST_TIMEOUT).build().ok()?;

    let body = SearchRequest::for_packages(query, limit);
    let json_body = serde_json::to_string(&body).ok()?;

    let url =
        format!("https://search.nixos.org/backend/latest-{BACKEND_VERSION}-{channel}/_search");

    let start = Instant::now();
    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .basic_auth(USER, Some(PASS))
        .body(json_body)
        .send()
        .ok()?;

    if !response.status().is_success() {
        eprintln!(
            "nixvis: search.nixos.org returned HTTP {} for channel '{}'",
            response.status(),
            channel
        );
        return None;
    }

    let es_resp: EsSearchResponse = response.json().ok()?;
    let elapsed = start.elapsed();
    eprintln!(
        "nixvis: fetched {} packages from search.nixos.org in {}ms",
        es_resp.hits.hits.len(),
        elapsed.as_millis()
    );

    let packages: Vec<PkgJson> = es_resp
        .hits
        .hits
        .into_iter()
        .map(|hit| pkg_from_source(hit._source))
        .collect();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    Some(IndexDoc {
        header: Header {
            schema: crate::model::SCHEMA_VERSION,
            nixpkgs_commit: "nixos-unstable (online)".to_string(),
            generated_ms: now.to_string(),
            package_count: packages.len() as u64,
        },
        packages,
    })
}

/// Convenience wrapper: search with defaults.
pub fn search_packages_default(query: &str, limit: usize) -> Option<IndexDoc> {
    search_packages(query, limit, DEFAULT_CHANNEL)
}

// ---------------------------------------------------------------------------
// Elasticsearch request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchRequest {
    query: EsQuery,
    size: usize,
}

#[derive(Debug, Serialize)]
struct EsQuery {
    #[serde(rename = "bool")]
    bool_: EsBool,
}

#[derive(Debug, Serialize)]
struct EsBool {
    filter: serde_json::Value,
    must: serde_json::Value,
}

impl SearchRequest {
    fn for_packages(query: &str, size: usize) -> Self {
        // Build a dis_max multi_match query identical to nh's `query::packages`.
        // We keep it simple: a bool filter on "type": "package" + a multi_match
        // across the most important fields.
        let filter = serde_json::json!({ "term": { "type": "package" } });
        let must = serde_json::json!({
            "dis_max": {
                "tie_breaker": 0.7,
                "queries": [
                    {
                        "multi_match": {
                            "query": query,
                            "fields": [
                                "package_attr_name^9",
                                "package_pname^6",
                                "package_programs^9",
                                "package_description^1.3",
                                "package_longDescription^1"
                            ],
                            "type": "cross_fields",
                            "operator": "and"
                        }
                    },
                    {
                        "wildcard": {
                            "package_attr_name": {
                                "value": format!("*{query}*"),
                                "case_insensitive": true
                            }
                        }
                    }
                ]
            }
        });

        SearchRequest {
            query: EsQuery {
                bool_: EsBool { filter, must },
            },
            size,
        }
    }
}

// ---------------------------------------------------------------------------
// Response parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct EsSearchResponse {
    hits: EsHitsContainer,
}

#[derive(Debug, Deserialize)]
struct EsHitsContainer {
    hits: Vec<EsHit>,
}

#[derive(Debug, Deserialize)]
struct EsHit {
    #[serde(rename = "_source")]
    _source: EsPackageSource,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
struct EsPackageSource {
    package_attr_name: String,
    #[serde(default)]
    package_pname: String,
    #[serde(default)]
    package_pversion: String,
    #[serde(default)]
    package_description: Option<String>,
    #[serde(default)]
    package_long_description: Option<String>,
    #[serde(default)]
    package_license: Vec<EsLicense>,
    #[serde(default)]
    package_homepage: Vec<String>,
    #[serde(default)]
    package_position: Option<String>,
    #[serde(default)]
    package_dep_count: u32,
}

#[derive(Debug, Deserialize)]
struct EsLicense {
    #[serde(default)]
    short_name: String,
    #[serde(default)]
    full_name: String,
}

fn pkg_from_source(src: EsPackageSource) -> PkgJson {
    let name = if src.package_pname.is_empty() {
        src.package_attr_name.clone()
    } else {
        format!("{}-{}", src.package_pname, src.package_pversion)
    };

    let synopsis = src.package_description.clone().unwrap_or_default();

    let description = src.package_long_description.clone().unwrap_or_default();

    let licenses: Vec<String> = src
        .package_license
        .iter()
        .map(|l| {
            if l.short_name.is_empty() {
                l.full_name.clone()
            } else {
                l.short_name.clone()
            }
        })
        .collect();

    let homepage = src.package_homepage.first().cloned().unwrap_or_default();

    PkgJson {
        id: 0, // Will be assigned by indexer
        name,
        attribute: src.package_attr_name,
        version: src.package_pversion,
        synopsis,
        description,
        homepage,
        licenses,
        file: (String::new(), 0),
        inputs: Vec::new(),
        propagated_inputs: Vec::new(),
        native_inputs: Vec::new(),
        store_path: String::new(),
        installed: false,
        flake: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hits() {
        let json = r#"{
            "hits": {
                "total": {"value": 1, "relation": "eq"},
                "hits": [
                    {
                        "_source": {
                            "package_attr_name": "hello",
                            "package_pname": "hello",
                            "package_pversion": "2.12.3",
                            "package_description": "A greeting program",
                            "package_homepage": ["https://example.com"],
                            "package_license": [{"shortName": "GPL-3.0", "fullName": "GNU GPLv3"}]
                        }
                    }
                ]
            }
        }"#;

        let resp: EsSearchResponse = serde_json::from_str(json).expect("parse ok");
        assert_eq!(resp.hits.hits.len(), 1);
        let src = &resp.hits.hits[0]._source;
        assert_eq!(src.package_attr_name, "hello");
        assert_eq!(src.package_pversion, "2.12.3");
    }

    #[test]
    fn test_pkg_from_source() {
        let src = EsPackageSource {
            package_attr_name: "hello".into(),
            package_pname: "hello".into(),
            package_pversion: "2.12.3".into(),
            package_description: Some("A greeting".into()),
            package_long_description: None,
            package_homepage: vec!["https://example.com".into()],
            package_license: vec![EsLicense {
                short_name: "GPL".into(),
                full_name: "GPLv3".into(),
            }],
            package_position: None,
            package_dep_count: 1,
        };
        let pkg = pkg_from_source(src);
        assert_eq!(pkg.name, "hello-2.12.3");
        assert_eq!(pkg.attribute, "hello");
        assert_eq!(pkg.version, "2.12.3");
    }
}
