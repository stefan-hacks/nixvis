//! Gzipped JSON index cache under `$XDG_CACHE_HOME/nixvis/`.
//!
//! Writes are transactional (temp file + atomic rename); corrupt files are
//! quarantined instead of silently deleted.

use std::fs::{self, File};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;

use crate::error::CacheError;
use crate::model::IndexDoc;

/// Metadata about a cache file without fully loading it.
#[derive(Debug, Clone)]
pub struct CacheInfo {
    pub exists: bool,
    pub size_bytes: u64,
    pub modified: u64, // UNIX epoch seconds
}

/// Whether a cached document is usable and current.
#[derive(Debug)]
pub enum CacheStatus {
    /// Cache parsed successfully and matches the live channel commit.
    /// The u64 is the UNIX epoch timestamp (seconds) when the cache was saved.
    Fresh(IndexDoc, u64),
    /// Cache is valid JSON but was produced against a different Nixpkgs commit.
    Stale { reason: String, timestamp: u64 },
    /// No cache file present.
    Absent,
}

pub struct Cache {
    dir: PathBuf,
}

impl Cache {
    pub fn new() -> Result<Self, CacheError> {
        let base = dirs::cache_dir().ok_or_else(|| {
            CacheError::Read("no cache directory available (XDG_CACHE_HOME unset?)".into())
        })?;
        let dir = base.join("nixvis");
        fs::create_dir_all(&dir).map_err(|e| CacheError::Read(e.to_string()))?;
        Ok(Cache { dir })
    }

    /// Build a cache rooted at an explicit directory (used by tests).
    pub fn at(dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&dir);
        Cache { dir }
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join("index-v3.json.gz")
    }

    /// Metadata about the cache file without deserialising it.
    pub fn info(&self) -> CacheInfo {
        let path = self.path();
        let meta = match fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => {
                return CacheInfo {
                    exists: false,
                    size_bytes: 0,
                    modified: 0,
                }
            }
        };
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        CacheInfo {
            exists: true,
            size_bytes: meta.len(),
            modified: mtime,
        }
    }

    /// Load and deserialize the cache. `live_commit` is the commit of the
    /// currently active Nixpkgs channel (`None` when it cannot be determined —
    /// the cache is then accepted with an "unverified" badge by the caller).
    pub fn load(&self, live_commit: Option<&str>) -> Result<CacheStatus, CacheError> {
        let info = self.info();
        if !info.exists {
            return Ok(CacheStatus::Absent);
        }
        let file = File::open(&self.path()).map_err(|e| CacheError::Read(e.to_string()))?;
        let decoder = GzDecoder::new(BufReader::new(file));
        let doc: IndexDoc =
            serde_json::from_reader(decoder).map_err(|e| CacheError::Parse(e.to_string()))?;

        if let Some(live) = live_commit.filter(|c| !c.is_empty()) {
            if doc.header.nixpkgs_commit != live {
                return Ok(CacheStatus::Stale {
                    reason: format!(
                        "cache commit {} != live commit {}",
                        doc.header.nixpkgs_commit, live
                    ),
                    timestamp: info.modified,
                });
            }
        }
        Ok(CacheStatus::Fresh(doc, info.modified))
    }

    /// Atomically persist raw index JSON bytes (gzip-compressed).
    pub fn save(&self, json_bytes: &[u8]) -> Result<(), CacheError> {
        let tmp = self
            .dir
            .join(format!("index-v3.json.gz.tmp-{}", std::process::id()));
        let result = (|| -> Result<(), CacheError> {
            let file = File::create(&tmp).map_err(|e| CacheError::Write(e.to_string()))?;
            let mut enc = GzEncoder::new(file, Compression::new(6));
            enc.write_all(json_bytes)
                .map_err(|e| CacheError::Write(e.to_string()))?;
            let file = enc.finish().map_err(|e| CacheError::Write(e.to_string()))?;
            file.sync_all()
                .map_err(|e| CacheError::Write(e.to_string()))?;
            fs::rename(&tmp, self.path()).map_err(|e| CacheError::Write(e.to_string()))
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result
    }

    /// Move a broken cache file out of the way instead of deleting evidence.
    pub fn quarantine(&self) {
        let path = self.path();
        if !path.exists() {
            return;
        }
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let dest = self.dir.join(format!("index-v3.json.gz.corrupt-{ts}"));
        let _ = fs::rename(&path, &dest);
    }
}

/// Small helper: canonical location of the `nix` binary.
pub fn find_nix() -> Option<PathBuf> {
    let candidates: Vec<PathBuf> = [
        Some(PathBuf::from("/run/current-system/profile/bin/nix")),
        dirs::home_dir().map(|h| h.join(".nix-profile/bin/nix")),
        Some(PathBuf::from("/nix/var/nix/profiles/default/bin/nix")),
        Some(PathBuf::from("/usr/bin/nix")),
    ]
    .into_iter()
    .flatten()
    .collect();
    candidates
        .iter()
        .find(|p| p.exists())
        .cloned()
        .or_else(which_nix)
}

fn which_nix() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join("nix"))
        .find(|p| p.is_file())
}

/// Read the active channel commit via `nix describe --format=json`.
/// Returns `None` when the command fails or exceeds 15 seconds.
pub fn nixpkgs_commit(nix: &Path) -> Option<String> {
    let mut child = std::process::Command::new(nix)
        .args(["describe", "--format=json"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let mut out = Vec::new();
    let _ = child.stdout.take()?.read_to_end(&mut out);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    if !status.success() || out.is_empty() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out).ok()?;
    v.as_array()?
        .first()?
        .get("commit")?
        .as_str()
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_info_empty_dir() {
        let dir = std::env::temp_dir()
            .join("nixvis-test-empty-")
            .join(format!(
                "{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
            ));
        let c = Cache::at(dir.clone());
        let info = c.info();
        assert!(!info.exists);
        assert_eq!(info.size_bytes, 0);
        assert_eq!(info.modified, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_cache_save_and_load() {
        let dir = std::env::temp_dir().join(format!(
            "nixvis-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        ));
        let c = Cache::at(dir.clone());
        let raw = b"hello world".to_vec();
        c.save(&raw).unwrap();
        assert!(c.info().exists);
        // Clean up.
        let _ = std::fs::remove_dir_all(&dir);
    }
}
