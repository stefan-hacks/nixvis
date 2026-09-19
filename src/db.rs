//! SQLite local database — the cache IS the database.
//!
//! On first run (or on rebuild) the tool fetches data from search.nixos.org,
//! bulk-inserts into an SQLite DB with indexed columns, and saves metadata.
//! Every subsequent run loads directly from SQLite — no re-parsing, no
//! re-building of in-memory Index structures.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension};

use crate::error::CacheError;
use crate::index::{Index, Package};
use crate::model::{Header, IndexDoc};
use std::sync::Arc;

/// Schema version stored in the database.
pub const DB_SCHEMA_VERSION: u32 = 4;

/// The database lives next to the old gzipped JSON cache.
pub fn db_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("nixvis")
        .join("index-v4.db")
}

/// Open (or create) the local SQLite database.
pub fn open_db() -> Result<Connection, CacheError> {
    let path = db_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let conn = Connection::open(&path).map_err(|e| CacheError::Write(e.to_string()))?;
    create_schema(&conn)?;
    Ok(conn)
}

fn create_schema(conn: &Connection) -> Result<(), CacheError> {
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS meta (
            key    TEXT PRIMARY KEY,
            value  TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS packages (
            id              INTEGER PRIMARY KEY,
            name            TEXT NOT NULL COLLATE NOCASE,
            version         TEXT NOT NULL DEFAULT '',
            synopsis        TEXT NOT NULL DEFAULT '',
            description     TEXT DEFAULT '',
            homepage        TEXT DEFAULT '',
            attribute       TEXT DEFAULT '',
            store_path      TEXT DEFAULT '',
            installed       INTEGER NOT NULL DEFAULT 0,
            flake           TEXT DEFAULT '',
            licenses        TEXT DEFAULT '',
            file_path       TEXT DEFAULT '',
            file_line       INTEGER DEFAULT 0
        );

        CREATE INDEX IF NOT EXISTS idx_pkg_name ON packages(name);
        CREATE INDEX IF NOT EXISTS idx_pkg_name_lower ON packages(LOWER(name));

        CREATE TABLE IF NOT EXISTS deps (
            pkg_id          INTEGER NOT NULL REFERENCES packages(id),
            dep_name        TEXT NOT NULL,
            kind            INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (pkg_id, dep_name, kind)
        );

        CREATE INDEX IF NOT EXISTS idx_deps_pkg ON deps(pkg_id);
        ",
    )
    .map_err(|e| CacheError::Write(e.to_string()))?;
    Ok(())
}

/// Metadata key constants.
mod keys {
    pub const SCHEMA_VERSION: &str = "schema_version";
    pub const SAVED_AT: &str = "saved_at";
    pub const NIXPKGS_COMMIT: &str = "nixpkgs_commit";
    pub const CHANNEL: &str = "channel";
    pub const GENERATED_AT: &str = "generated_at";
    pub const PACKAGE_COUNT: &str = "package_count";
}

/// Save a full index document into SQLite (bulk insert inside a transaction).
pub fn save_index(conn: &mut Connection, doc: &IndexDoc) -> Result<(), CacheError> {
    let tx = conn
        .transaction()
        .map_err(|e| CacheError::Write(e.to_string()))?;

    // Clear old data.
    tx.execute("DELETE FROM deps", [])
        .map_err(|e| CacheError::Write(e.to_string()))?;
    tx.execute("DELETE FROM packages", [])
        .map_err(|e| CacheError::Write(e.to_string()))?;
    tx.execute("DELETE FROM meta", [])
        .map_err(|e| CacheError::Write(e.to_string()))?;

    // Insert packages.
    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO packages
                 (id, name, version, synopsis, description, homepage, attribute, store_path, installed, flake, licenses, file_path, file_line)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            )
            .map_err(|e| CacheError::Write(e.to_string()))?;
        for pkg in &doc.packages {
            let licenses = pkg.licenses.join(", ");
            let installed_i = if pkg.installed { 1 } else { 0 };
            stmt.execute([
                &pkg.id.to_string(),
                &pkg.name,
                &pkg.version,
                &pkg.synopsis,
                &pkg.description,
                &pkg.homepage,
                &pkg.attribute,
                &pkg.store_path,
                &installed_i.to_string(),
                &pkg.flake,
                &licenses,
                &pkg.file.0,
                &pkg.file.1.to_string(),
            ])
            .map_err(|e| CacheError::Write(e.to_string()))?;
        }
    }

    // Insert deps (by name; resolved to IDs on load).
    {
        let mut stmt = tx
            .prepare("INSERT INTO deps (pkg_id, dep_name, kind) VALUES (?1, ?2, ?3)")
            .map_err(|e| CacheError::Write(e.to_string()))?;
        for pkg in &doc.packages {
            for dep in &pkg.inputs {
                stmt.execute(rusqlite::params![&pkg.id, dep.as_str(), 0])
                    .map_err(|e| CacheError::Write(e.to_string()))?;
            }
            for dep in &pkg.propagated_inputs {
                stmt.execute(rusqlite::params![&pkg.id, dep.as_str(), 1])
                    .map_err(|e| CacheError::Write(e.to_string()))?;
            }
            for dep in &pkg.native_inputs {
                stmt.execute(rusqlite::params![&pkg.id, dep.as_str(), 2])
                    .map_err(|e| CacheError::Write(e.to_string()))?;
            }
        }
    }

    // Metadata.
    {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut insert_meta = tx
            .prepare("INSERT INTO meta (key, value) VALUES (?1, ?2)")
            .map_err(|e| CacheError::Write(e.to_string()))?;
        insert_meta
            .execute([keys::SCHEMA_VERSION, &DB_SCHEMA_VERSION.to_string()])
            .map_err(|e| CacheError::Write(e.to_string()))?;
        insert_meta
            .execute([keys::SAVED_AT, &now.to_string()])
            .map_err(|e| CacheError::Write(e.to_string()))?;
        insert_meta
            .execute([keys::NIXPKGS_COMMIT, &doc.header.nixpkgs_commit])
            .map_err(|e| CacheError::Write(e.to_string()))?;
        insert_meta
            .execute([keys::CHANNEL, "nixos-unstable"])
            .map_err(|e| CacheError::Write(e.to_string()))?;
        insert_meta
            .execute([keys::GENERATED_AT, &doc.header.generated_ms])
            .map_err(|e| CacheError::Write(e.to_string()))?;
        insert_meta
            .execute([keys::PACKAGE_COUNT, &doc.header.package_count.to_string()])
            .map_err(|e| CacheError::Write(e.to_string()))?;
        // drop insert_meta before tx.commit()
    }

    tx.commit().map_err(|e| CacheError::Write(e.to_string()))?;
    Ok(())
}

/// Check whether a usable database exists and return its metadata.
///
/// Returns `None` if the DB is missing, empty, or on an unsupported schema.
pub fn db_info() -> Option<DbInfo> {
    let path = db_path();
    if !path.exists() {
        return None;
    }
    let meta = fs::metadata(&path).ok()?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let size_bytes = meta.len();

    let conn = Connection::open(&path).ok()?;
    let schema: i64 = conn
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            [keys::SCHEMA_VERSION],
            |row| row.get(0),
        )
        .optional()
        .ok()??;
    if schema != DB_SCHEMA_VERSION as i64 {
        return None;
    }

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM packages", [], |row| row.get(0))
        .optional()
        .ok()??;

    let nixpkgs_commit: String = conn
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            [keys::NIXPKGS_COMMIT],
            |row| row.get(0),
        )
        .optional()
        .ok()??;

    let channel: String = conn
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            [keys::CHANNEL],
            |row| row.get(0),
        )
        .optional()
        .ok()??;

    Some(DbInfo {
        package_count: count as u64,
        size_bytes,
        modified,
        nixpkgs_commit,
        channel,
    })
}

/// Build an in-memory `Index` directly from the SQLite DB.
pub fn load_index_from_db() -> Result<(Index, Header), CacheError> {
    let conn = Connection::open(db_path()).map_err(|e| CacheError::Read(e.to_string()))?;

    // Load metadata.
    let mut meta = HashMap::new();
    {
        let mut stmt = conn
            .prepare("SELECT key, value FROM meta")
            .map_err(|e| CacheError::Read(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<usize, String>(0)?, row.get::<usize, String>(1)?))
            })
            .map_err(|e| CacheError::Read(e.to_string()))?;
        for row in rows {
            let (k, v) = row.map_err(|e| CacheError::Read(e.to_string()))?;
            meta.insert(k, v);
        }
    }

    let header = Header {
        schema: meta
            .get(keys::SCHEMA_VERSION)
            .and_then(|s| s.parse().ok())
            .unwrap_or(DB_SCHEMA_VERSION),
        nixpkgs_commit: meta.get(keys::NIXPKGS_COMMIT).cloned().unwrap_or_default(),
        generated_ms: meta.get(keys::GENERATED_AT).cloned().unwrap_or_default(),
        package_count: meta
            .get(keys::PACKAGE_COUNT)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
    };

    // Load packages.
    let mut packages: Vec<Package> = Vec::new();
    let mut name_to_id: HashMap<Arc<str>, u32> = HashMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT id, name, version, synopsis, description, homepage, attribute, store_path, installed, flake, licenses, file_path, file_line FROM packages ORDER BY id"
            )
            .map_err(|e| CacheError::Read(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                let id: u32 = row.get(0)?;
                let name: String = row.get(1)?;
                let version: String = row.get(2)?;
                let synopsis: String = row.get(3)?;
                let description: String = row.get(4)?;
                let homepage: String = row.get(5)?;
                let attribute: String = row.get(6)?;
                let store_path: String = row.get(7)?;
                let installed: i64 = row.get(8)?;
                let flake: String = row.get(9)?;
                let licenses: String = row.get(10)?;
                let file_path: String = row.get(11)?;
                let file_line: i64 = row.get(12)?;
                let licenses_vec: Vec<Arc<str>> = licenses
                    .split(", ")
                    .filter(|s| !s.is_empty())
                    .map(Arc::from)
                    .collect();
                Ok((
                    id,
                    name,
                    version,
                    synopsis,
                    description,
                    homepage,
                    attribute,
                    store_path,
                    installed != 0,
                    flake,
                    licenses_vec,
                    file_path,
                    file_line,
                ))
            })
            .map_err(|e| CacheError::Read(e.to_string()))?;
        for row in rows {
            let (
                id,
                name,
                version,
                synopsis,
                description,
                homepage,
                attribute,
                store_path,
                installed,
                flake,
                licenses,
                file_path,
                file_line,
            ) = row.map_err(|e| CacheError::Read(e.to_string()))?;
            name_to_id.insert(Arc::from(name.clone()), id);
            packages.push(Package {
                id,
                name: Arc::from(name),
                version: Arc::from(version),
                synopsis: Arc::from(synopsis),
                description: Arc::from(description),
                homepage: Arc::from(homepage),
                licenses: licenses.into(),
                file: Arc::from(file_path),
                line: file_line as u64,
                attribute: Arc::from(attribute),
                store_path: Arc::from(store_path),
                installed,
                flake: Arc::from(flake),
                inputs: Arc::from([]),
                propagated: Arc::from([]),
                native: Arc::from([]),
            });
        }
    }

    // Resolve dependency names to IDs.
    {
        let mut stmt = conn
            .prepare("SELECT pkg_id, dep_name, kind FROM deps ORDER BY pkg_id, kind")
            .map_err(|e| CacheError::Read(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                let pkg_id: u32 = row.get(0)?;
                let dep_name: String = row.get(1)?;
                let kind: i64 = row.get(2)?;
                Ok((pkg_id, dep_name, kind))
            })
            .map_err(|e| CacheError::Read(e.to_string()))?;

        let mut inputs_map: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut propagated_map: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut native_map: HashMap<u32, Vec<u32>> = HashMap::new();

        for row in rows {
            let (pkg_id, dep_name, kind) = row.map_err(|e| CacheError::Read(e.to_string()))?;
            if let Some(&dep_id) = name_to_id.get(dep_name.as_str()) {
                match kind {
                    0 => inputs_map.entry(pkg_id).or_default().push(dep_id),
                    1 => propagated_map.entry(pkg_id).or_default().push(dep_id),
                    2 => native_map.entry(pkg_id).or_default().push(dep_id),
                    _ => {}
                }
            }
        }

        for pkg in &mut packages {
            if let Some(v) = inputs_map.remove(&pkg.id) {
                pkg.inputs = Arc::from(v);
            }
            if let Some(v) = propagated_map.remove(&pkg.id) {
                pkg.propagated = Arc::from(v);
            }
            if let Some(v) = native_map.remove(&pkg.id) {
                pkg.native = Arc::from(v);
            }
        }
    }

    let index = Index::from_packages(packages, header.nixpkgs_commit.clone())
        .map_err(|e| CacheError::Read(e.to_string()))?;
    Ok((index, header))
}

/// Info about an existing database file (without loading data).
#[derive(Debug, Clone)]
pub struct DbInfo {
    pub package_count: u64,
    pub size_bytes: u64,
    pub modified: u64, // UNIX epoch seconds
    pub nixpkgs_commit: String,
    pub channel: String,
}

impl DbInfo {
    pub fn size_mb(&self) -> f64 {
        self.size_bytes as f64 / (1024.0 * 1024.0)
    }

    pub fn age_text(&self) -> String {
        if self.modified == 0 {
            return "unknown date".to_string();
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let diff = now.saturating_sub(self.modified);
        let days = diff / 86400;
        let hrs = (diff % 86400) / 3600;
        let mins = (diff % 3600) / 60;
        if days > 0 {
            format!("{days}d ago")
        } else if hrs > 0 {
            format!("{hrs}h ago")
        } else if mins > 0 {
            format!("{mins}m ago")
        } else {
            format!("{diff}s ago")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_db_info_empty() {
        let dir = std::env::temp_dir().join(format!(
            "nixvis-test-db-{}",
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
        ));
        let _ = fs::create_dir_all(&dir);
        // Temporarily override db_path for testing would require env var or refactoring
        // For now, just test the age_text helper.
        let info = DbInfo {
            package_count: 0,
            size_bytes: 0,
            modified: 0,
            nixpkgs_commit: "".to_string(),
            channel: "".to_string(),
        };
        assert_eq!(info.age_text(), "unknown date");
    }
}
