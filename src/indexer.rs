//! The indexer: loads the embedded JSON index or builds via `nix search --json`.
//!
//! # Strategy
//! 1. **Embedded JSON** (`data/nix-index.json`) — used when Nix is unavailable or
//!    when the user forces a fallback.
//! 2. **Live `nix search --json`** — tries first when `nix` is found on PATH.
//!
//! Progress events are emitted so the TUI can show a spinner during the
//! potentially-long evaluation.

use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::cache::{find_nix, nixpkgs_commit, Cache, CacheStatus};
use crate::error::IndexerError;
use crate::index::Index;

/// Embed the sample Nix index JSON shipped with the binary.
const EMBEDDED_INDEX: &str = include_str!("../data/nix-index.json");

/// Hard timeout for a full cold index build (nixpkgs evaluation can take
/// minutes on a fresh cache).
const TIMEOUT: Duration = Duration::from_secs(900);
/// Upper bound on the JSON document the script may emit (~20 MB observed).
const MAX_OUTPUT: u64 = 256 * 1024 * 1024;

pub enum IndexEvent {
    /// `PROGRESS <done> <total>` line seen on stderr.
    Progress { done: u64, total: u64 },
    /// A valid index is available (from cache or a finished build).
    Ready {
        index: std::sync::Arc<crate::index::Index>,
        fresh: bool,
        unkeyed: bool,
    },
    /// The load/build failed; `msg` is safe to show in the status bar.
    Failed { msg: String },
}

/// Cancellation token shared with the loader/indexer thread.
pub type Cancel = Arc<AtomicBool>;

/// Load the index from cache or build it; runs on its own thread and reports
/// through `tx`. Shared by the TUI and the web server.
pub fn start_loader(tx: Sender<IndexEvent>, cancel: Cancel, force: bool) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("nixvis-loader".into())
        .spawn(move || {
            let now_ms = || -> u64 {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0)
            };
            let nix = find_nix();
            let commit = nix.as_deref().and_then(nixpkgs_commit);

            if !force {
                if let Ok(cache) = Cache::new() {
                    match cache.load(commit.as_deref()) {
                        Ok(CacheStatus::Fresh(doc, _)) => match Index::from_doc(doc, now_ms()) {
                            Ok(index) => {
                                let _ = tx.send(IndexEvent::Ready {
                                    index: Arc::new(index),
                                    fresh: true,
                                    unkeyed: commit.is_none(),
                                });
                                return;
                            }
                            Err(e) => {
                                let _ = tx.send(IndexEvent::Failed {
                                    msg: format!("cached index rejected ({e}); rebuilding"),
                                });
                                cache.quarantine();
                            }
                        },
                        Ok(CacheStatus::Stale { reason, .. }) => {
                            let _ = tx.send(IndexEvent::Failed {
                                msg: format!("cache stale ({reason}); rebuilding"),
                            });
                        }
                        Ok(CacheStatus::Absent) => {
                            let _ = tx.send(IndexEvent::Progress { done: 0, total: 0 });
                        }
                        Err(e) => {
                            let _ = tx.send(IndexEvent::Failed {
                                msg: format!("cache unreadable ({e}); rebuilding"),
                            });
                            cache.quarantine();
                        }
                    }
                }
            }

            // Rebuild path: run the indexer, then save the cache and SQLite DB.
            match build(commit.as_deref(), &cancel, &tx) {
                Ok((doc, raw, _)) => {
                    // Save old gzipped JSON cache (kept for compatibility).
                    if let Ok(cache) = Cache::new() {
                        if let Err(e) = cache.save(&raw) {
                            let _ = tx.send(IndexEvent::Failed {
                                msg: format!("cache save failed: {e}"),
                            });
                        }
                    }
                    // Save to SQLite DB (the new fast path).
                    match crate::db::open_db() {
                        Ok(mut conn) => {
                            if let Err(e) = crate::db::save_index(&mut conn, &doc) {
                                eprintln!("DB save warning: {e}");
                            }
                        }
                        Err(e) => eprintln!("DB open warning: {e}"),
                    }
                    match Index::from_doc(doc, now_ms()) {
                        Ok(index) => {
                            let _ = tx.send(IndexEvent::Ready {
                                index: Arc::new(index),
                                fresh: false,
                                unkeyed: commit.is_none(),
                            });
                        }
                        Err(e) => {
                            let _ = tx.send(IndexEvent::Failed {
                                msg: format!("indexer output invalid: {e}"),
                            });
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(IndexEvent::Failed {
                        msg: format!("index build failed: {e}"),
                    });
                }
            }
        })
        .expect("failed to spawn loader thread")
}

/// Run `nix search --json` (or load embedded JSON as fallback) and return the
/// parsed document plus raw bytes (for the cache). Blocking; call from a worker
/// thread. Progress events are relayed through `progress`.
pub fn build(
    _commit: Option<&str>,
    cancel: &Cancel,
    progress: &Sender<IndexEvent>,
) -> Result<(crate::model::IndexDoc, Vec<u8>, String), IndexerError> {
    let _started = Instant::now();

    // Try fast online search first (sub-second fuzzy results from search.nixos.org).
    if let Some(doc) = crate::nixos_search::search_packages_default("", 50_000) {
        let raw = serde_json::to_vec(&doc)
            .map_err(|e| IndexerError::Exited(format!("serialize: {e}")))?;
        let commit = doc.header.nixpkgs_commit.clone();
        return Ok((doc, raw, commit));
    }

    // Fallback 1: local `nix search --json` (slow, but works offline).
    if let Some(nix_path) = find_nix() {
        let _ = progress.send(IndexEvent::Progress {
            done: 5,
            total: 100,
        });
        match run_nix_search(&nix_path, cancel, progress) {
            Ok(doc) => {
                let raw = serde_json::to_vec(&doc)
                    .map_err(|e| IndexerError::Exited(format!("serialize: {e}")))?;
                let commit = doc.header.nixpkgs_commit.clone();
                return Ok((doc, raw, commit));
            }
            Err(e) => {
                let _ = progress.send(IndexEvent::Failed {
                    msg: format!("nix search failed ({e}); falling back to embedded JSON"),
                });
            }
        }
    }

    // Fallback 2: embedded JSON (works without Nix or internet).
    let _ = progress.send(IndexEvent::Progress {
        done: 50,
        total: 100,
    });
    let raw = EMBEDDED_INDEX.as_bytes();
    let doc: crate::model::IndexDoc = serde_json::from_slice(raw)
        .map_err(|e| IndexerError::Exited(format!("cannot parse embedded index: {e}")))?;
    let _ = progress.send(IndexEvent::Progress {
        done: 100,
        total: 100,
    });
    let commit = doc.header.nixpkgs_commit.clone();
    Ok((doc, raw.to_vec(), commit))
}

/// Spawn `nix search --json` and convert its output into an `IndexDoc`.
fn run_nix_search(
    nix: &PathBuf,
    cancel: &Cancel,
    progress: &Sender<IndexEvent>,
) -> Result<crate::model::IndexDoc, IndexerError> {
    // nix search evaluates the whole nixpkgs set; cap at 50k results.
    let mut child = Command::new(nix)
        .args(["search", "nixpkgs", "", "--json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| IndexerError::Exited(format!("cannot spawn nix search: {e}")))?;

    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");

    // Drain stderr in a background thread to prevent the child from blocking.
    let progress_tx = progress.clone();
    let stderr_reader: JoinHandle<()> = std::thread::Builder::new()
        .name("nixvis-stderr".into())
        .spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                // nix search prints "evaluating '...'" lines to stderr.
                if line.starts_with("evaluating") {
                    let _ = progress_tx.send(IndexEvent::Progress {
                        done: 10,
                        total: 100,
                    });
                }
            }
        })
        .expect("failed to spawn stderr reader");

    // Read stdout with timeout.
    let mut out = Vec::new();
    let mut buf = BufReader::new(stdout);
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stderr_reader.join();
            return Err(IndexerError::Exited("cancelled".into()));
        }
        match buf.read_to_end(&mut out) {
            Ok(0) => break,
            Ok(_) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stderr_reader.join();
                    return Err(IndexerError::Timeout(TIMEOUT.as_secs()));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stderr_reader.join();
                return Err(IndexerError::Exited(format!("read error: {e}")));
            }
        }
    }

    let _ = stderr_reader.join();
    let status = child
        .wait()
        .map_err(|e| IndexerError::Exited(format!("wait error: {e}")))?;
    if !status.success() {
        return Err(IndexerError::Exited(format!(
            "nix search exited with {status}"
        )));
    }
    if out.is_empty() {
        return Err(IndexerError::Exited("nix search produced no output".into()));
    }
    if out.len() as u64 > MAX_OUTPUT {
        return Err(IndexerError::Exited(format!(
            "nix search output too large ({} bytes)",
            out.len()
        )));
    }

    let _ = progress.send(IndexEvent::Progress {
        done: 80,
        total: 100,
    });
    let nix_pkgs: serde_json::Value = serde_json::from_slice(&out)
        .map_err(|e| IndexerError::Exited(format!("invalid JSON from nix search: {e}")))?;

    let doc = parse_nix_search_json(nix_pkgs)?;
    let _ = progress.send(IndexEvent::Progress {
        done: 100,
        total: 100,
    });
    Ok(doc)
}

/// Convert `nix search --json` output into our `IndexDoc` format.
///
/// `nix search` produces:
/// ```json
/// {
///   "legacyPackages.x86_64-linux.hello": {
///     "description": "...",
///     "pname": "hello",
///     "version": "2.12",
///     "name": "hello-2.12"
///   }
/// }
/// ```
fn parse_nix_search_json(v: serde_json::Value) -> Result<crate::model::IndexDoc, IndexerError> {
    let obj = v
        .as_object()
        .ok_or_else(|| IndexerError::Exited("nix search output is not an object".into()))?;

    let mut packages = Vec::with_capacity(obj.len().min(50_000));
    let mut id = 0u32;

    for (attr_path, pkg_val) in obj.iter().take(50_000) {
        let Some(pkg) = pkg_val.as_object() else {
            continue;
        };

        let name = pkg
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(attr_path);

        let description = pkg
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let version = pkg.get("version").and_then(|v| v.as_str()).unwrap_or("");
        let pname = pkg.get("pname").and_then(|v| v.as_str()).unwrap_or(name);

        packages.push(crate::model::PkgJson {
            id,
            name: pname.to_string(),
            attribute: attr_path.to_string(),
            version: version.to_string(),
            synopsis: description.to_string(),
            description: String::new(),
            licenses: vec![],
            homepage: String::new(),
            file: (String::new(), 0),
            inputs: vec![],
            propagated_inputs: vec![],
            native_inputs: vec![],
            store_path: String::new(),
            installed: false,
            flake: String::new(),
        });
        id += 1;
    }

    let header = crate::model::Header {
        schema: crate::model::SCHEMA_VERSION,
        nixpkgs_commit: String::new(),
        generated_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
            .to_string(),
        package_count: packages.len() as u64,
    };

    Ok(crate::model::IndexDoc { header, packages })
}
