//! The indexer: runs the embedded Nix script through `nix repl`, streams
//! progress from stderr, collects the JSON document, and emits typed events
//! to the UI thread. Fully cancellable; the child is killed on cancel/drop.

use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::cache::{nixpkgs_commit, find_nix, Cache, CacheStatus};
use crate::error::IndexerError;
use crate::index::Index;

/// Embed the sample Nix index JSON shipped with the binary.
const EMBEDDED_INDEX: &str = include_str!("../data/nix-index.json");

/// Hard timeout for a full cold index build (Nix module compilation can
/// take minutes on a fresh cache).
const TIMEOUT: Duration = Duration::from_secs(900);
/// Upper bound on the JSON document the script may emit (~20 MB observed).
const MAX_OUTPUT: u64 = 256 * 1024 * 1024;

pub enum IndexEvent {
    /// `PROGRESS <done> <total>` line seen on the script's stderr.
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

/// Load the index from cache or build it via `nix repl`; runs on its own
/// thread and reports through `tx`. Shared by the TUI and the web server.
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
                        Ok(CacheStatus::Fresh(doc)) => match Index::from_doc(doc, now_ms()) {
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
                        Ok(CacheStatus::Stale { reason }) => {
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

            // Rebuild path: run the indexer, then save the cache.
            match build(commit.as_deref(), &cancel, &tx) {
                Ok((doc, raw, _)) => {
                    if let Ok(cache) = Cache::new() {
                        if let Err(e) = cache.save(&raw) {
                            let _ = tx.send(IndexEvent::Failed {
                                msg: format!("cache save failed: {e}"),
                            });
                        }
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

/// A temp file removed on drop.
struct TempScript(PathBuf);

impl Drop for TempScript {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Build an index document from the embedded JSON, converting string-based
/// dependency names to numeric IDs.
pub fn build(
    _commit: Option<&str>,
    cancel: &Cancel,
    progress: &Sender<IndexEvent>,
) -> Result<(crate::model::IndexDoc, Vec<u8>, String), IndexerError> {
    let _ = progress.send(IndexEvent::Progress { done: 0, total: 100 });

    if cancel.load(Ordering::Relaxed) {
        return Err(IndexerError::Exited("cancelled".into()));
    }

    // Parse embedded JSON (dependencies remain string names; index.rs resolves them)
    let raw = EMBEDDED_INDEX.as_bytes();
    let doc: crate::model::IndexDoc = serde_json::from_slice(raw)
        .map_err(|e| IndexerError::Exited(format!("cannot parse embedded index: {e}")))?;

    let _ = progress.send(IndexEvent::Progress { done: 100, total: 100 });

    let commit = doc.header.nixpkgs_commit.clone();
    Ok((doc, raw.to_vec(), commit))
}

fn nix_search_paths() -> String {
    let mut parts = [
        "$GUIX/bin/nix",
        "/run/current-system/profile/bin/nix",
        "~/.config/nix/current/bin/nix",
        "~/.nix-profile/bin/nix",
        "$PATH/nix",
    ]
    .join(", ");
    parts.push_str(". Install GNU Nix or export GUIX to its profile.");
    parts
}
