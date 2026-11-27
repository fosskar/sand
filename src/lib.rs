//! sand: Semantic memory for AI agents
//!
//! A local-first, MCP-native vector database for AI agent memory.
//!
//! ## Features
//!
//! - **Embedded storage** - LanceDB-powered, directory-based, no server required
//! - **Local embeddings** - Uses `all-MiniLM-L6-v2` locally, no API keys needed
//! - **MCP-native** - 4 tools for seamless LLM integration
//! - **Project-aware** - Scoped memories with automatic project detection
//! - **Auto-chunking** - Long content is automatically chunked for better search

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;

pub mod backend;
pub mod chunker;
pub mod cli;
pub mod compaction;
pub mod consolidation;
pub mod db;
pub mod document;
pub mod embedder;
pub mod error;
pub mod graph;
pub mod item;
pub mod mcp;
pub mod retry;

pub use chunker::{ChunkResult, ChunkingConfig, chunk_content};
pub use db::Database;
pub use document::ContentType;
pub use embedder::{EMBEDDING_DIM, Embedder, EmbeddingModel};
pub use error::{Result, SandError};
pub use item::{Chunk, ConflictInfo, Item, ItemFilters, SearchResult, StoreResult};
pub use retry::{RetryConfig, with_retry};

/// Scope for storing items
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StoreScope {
    /// Store in project-local scope (with project_id)
    #[default]
    Project,
    /// Store in global scope (no project_id)
    Global,
}

impl std::str::FromStr for StoreScope {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "project" => Ok(StoreScope::Project),
            "global" => Ok(StoreScope::Global),
            _ => Err(format!(
                "Invalid store scope: {}. Use 'project' or 'global'",
                s
            )),
        }
    }
}

impl std::fmt::Display for StoreScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreScope::Project => write!(f, "project"),
            StoreScope::Global => write!(f, "global"),
        }
    }
}

/// Scope for listing items (recall always searches all with boosting)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ListScope {
    /// List only project-local items
    #[default]
    Project,
    /// List only global items
    Global,
    /// List all items
    All,
}

impl std::str::FromStr for ListScope {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "project" => Ok(ListScope::Project),
            "global" => Ok(ListScope::Global),
            "all" => Ok(ListScope::All),
            _ => Err(format!(
                "Invalid list scope: {}. Use 'project', 'global', or 'all'",
                s
            )),
        }
    }
}

impl std::fmt::Display for ListScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ListScope::Project => write!(f, "project"),
            ListScope::Global => write!(f, "global"),
            ListScope::All => write!(f, "all"),
        }
    }
}

/// Read the setting `SAND_<name>` from the environment.
///
/// Falls back to `SEDIMENT_<name>`, the name sand had before 0.6.0, when `SAND_<name>` is unset.
pub fn setting(name: &str) -> Option<String> {
    if let Ok(value) = std::env::var(format!("SAND_{name}")) {
        return Some(value);
    }
    let value = std::env::var(format!("SEDIMENT_{name}")).ok()?;
    deprecated(&format!("SEDIMENT_{name}"), &format!("SAND_{name}"));
    Some(value)
}

/// Tell the user on stderr, once per process for each `old`, that a Sediment fallback goes in 1.0.
///
/// Never print to stdout: it carries CLI JSON output and MCP JSON-RPC.
pub fn deprecated(old: &str, new: &str) {
    static SEEN: std::sync::Mutex<std::collections::BTreeSet<String>> =
        std::sync::Mutex::new(std::collections::BTreeSet::new());
    if SEEN.lock().unwrap().insert(old.to_string()) {
        eprintln!("sand: {old} is deprecated and will be removed in 1.0, use {new}");
    }
}

/// Get the central database path.
///
/// Returns the path in the `SAND_DB` setting, else `~/.sand/data`, else Sediment's
/// `~/.sediment/data` when only that exists.
/// Note: LanceDB uses a directory, not a single file.
pub fn central_db_path() -> PathBuf {
    if let Some(path) = setting("DB") {
        return PathBuf::from(path);
    }

    default_db_path(&dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")))
}

fn default_db_path(home: &Path) -> PathBuf {
    let legacy = home.join(".sediment").join("data");
    if !home.join(".sand").exists() && legacy.exists() {
        deprecated("~/.sediment/data", "~/.sand/data");
        return legacy;
    }
    home.join(".sand").join("data")
}

/// Project configuration Sediment wrote to `.sediment/config`; sand only reads it.
#[derive(Debug, Deserialize)]
struct ProjectConfig {
    project_id: String,
    /// Set during Sediment's UUID→git migration; cleared after LanceDB items were updated
    #[serde(default)]
    migrated_from: Option<String>,
}

/// Derive a stable project ID from the git repository's initial (root) commit hash.
///
/// Returns `Ok(Some(hash))` if the project root is inside a git repo with at least one commit.
/// Returns `Ok(None)` if git is not installed, the directory is not a git repo, or there are no commits.
pub fn derive_git_root_commit(project_root: &Path) -> std::io::Result<Option<String>> {
    // Check for shallow clone — root commit in shallow history is not the true root
    let shallow_check = match Command::new("git")
        .args(["rev-parse", "--is-shallow-repository"])
        .current_dir(project_root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
    {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };

    if shallow_check.status.success() {
        let stdout = String::from_utf8_lossy(&shallow_check.stdout);
        if stdout.trim() == "true" {
            return Ok(None);
        }
    }

    let output = match Command::new("git")
        .args(["rev-list", "--max-parents=0", "HEAD"])
        .current_dir(project_root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
    {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };

    if !output.status.success() {
        return Ok(None);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let hash = stdout.lines().next().unwrap_or("").trim();

    // Validate: must be non-empty hex, at most 64 chars (covers SHA-1 40 and SHA-256 64)
    if !hash.is_empty() && hash.len() <= 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(Some(hash.to_string()))
    } else {
        Ok(None)
    }
}

/// The project a working directory belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    /// Directory the id belongs to
    pub root: PathBuf,
    pub id: String,
    /// UUID of a directory that became a git repository; its items still have to move to `id`
    pub migrate_from: Option<String>,
}

/// Find the project of `dir`.
///
/// In a git repository the id is the root commit and nothing is stored. Elsewhere it is a UUID
/// in the `projects` table of `access_db`, owned by the nearest ancestor that has one. Ids from
/// Sediment's `.sediment/config` files are imported into the table; the files are only read.
pub fn resolve_project(dir: &Path, access_db: &Path) -> Result<Project> {
    let dir = std::fs::canonicalize(dir)?;
    let table = ProjectTable::open(access_db)?;
    let state_dir = access_db.parent().unwrap_or(Path::new("."));

    if let Some(id) = derive_git_root_commit(&dir)? {
        let Some(git_root) = dir.ancestors().find(|d| d.join(".git").exists()) else {
            return Ok(Project {
                root: dir,
                id,
                migrate_from: None,
            });
        };
        // a uuid stored below or at the git root belongs to the time before the repository
        let mut migrate_from = None;
        for level in dir.ancestors() {
            let stored = match table.get(level)? {
                Some(stored) => Some(stored),
                None => legacy_config(level, state_dir)?
                    .map(|config| config.migrated_from.unwrap_or(config.project_id)),
            };
            if let Some(stored) = stored {
                if stored != id {
                    table.insert(level, &stored)?;
                    migrate_from = Some(stored);
                }
                break;
            }
            if level == git_root {
                break;
            }
        }
        return Ok(Project {
            root: git_root.to_path_buf(),
            id,
            migrate_from,
        });
    }

    let mut root = dir.as_path();
    for level in dir.ancestors() {
        if let Some(id) = table.get(level)? {
            return Ok(Project {
                root: level.to_path_buf(),
                id,
                migrate_from: None,
            });
        }
        if let Some(config) = legacy_config(level, state_dir)? {
            return Ok(Project {
                root: level.to_path_buf(),
                id: table.insert(level, &config.project_id)?,
                migrate_from: None,
            });
        }
        // sediment made these directories project roots
        if level.join(".sediment").is_dir() || level.join(".git").exists() {
            root = level;
            break;
        }
    }
    Ok(Project {
        root: root.to_path_buf(),
        id: table.insert(root, &Uuid::new_v4().to_string())?,
        migrate_from: None,
    })
}

/// Record that the items of `old_id` were moved to `new_id`.
pub fn finish_project_migration(access_db: &Path, old_id: &str, new_id: &str) -> Result<()> {
    ProjectTable::open(access_db)?
        .conn
        .execute(
            "UPDATE projects SET project_id = ?1 WHERE project_id = ?2",
            rusqlite::params![new_id, old_id],
        )
        .map_err(|e| SandError::Database(format!("Failed to update project ids: {}", e)))?;
    Ok(())
}

/// Lowercase hex SHA-256 of `bytes`.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Read the config Sediment kept for `dir`: in its central state, else in the directory.
fn legacy_config(dir: &Path, state_dir: &Path) -> Result<Option<ProjectConfig>> {
    let key = sha256_hex(dir.as_os_str().as_encoded_bytes());
    let central = state_dir
        .join("projects")
        .join(key)
        .join(".sediment/config");
    for path in [central, dir.join(".sediment/config")] {
        if path.exists()
            && let Ok(config) = serde_json::from_str(&std::fs::read_to_string(&path)?)
        {
            return Ok(Some(config));
        }
    }
    Ok(None)
}

/// Project ids of directories outside git, in `access.db`.
struct ProjectTable {
    conn: rusqlite::Connection,
}

impl ProjectTable {
    fn open(access_db: &Path) -> Result<Self> {
        if let Some(parent) = access_db.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = rusqlite::Connection::open(access_db)
            .map_err(|e| SandError::Database(format!("Failed to open project table: {}", e)))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;
             CREATE TABLE IF NOT EXISTS projects (
                 path TEXT PRIMARY KEY,
                 project_id TEXT NOT NULL,
                 created_at INTEGER NOT NULL
             );",
        )
        .map_err(|e| SandError::Database(format!("Failed to create projects table: {}", e)))?;
        Ok(Self { conn })
    }

    fn get(&self, path: &Path) -> Result<Option<String>> {
        use rusqlite::OptionalExtension;
        self.conn
            .query_row(
                "SELECT project_id FROM projects WHERE path = ?1",
                [path.to_string_lossy()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| SandError::Database(format!("Failed to read project id: {}", e)))
    }

    /// Store `id` for `path` unless a concurrent process stored one first; return the stored id.
    fn insert(&self, path: &Path, id: &str) -> Result<String> {
        self.conn
            .execute(
                "INSERT OR IGNORE INTO projects (path, project_id, created_at) VALUES (?1, ?2, ?3)",
                rusqlite::params![path.to_string_lossy(), id, chrono::Utc::now().timestamp()],
            )
            .map_err(|e| SandError::Database(format!("Failed to store project id: {}", e)))?;
        Ok(self.get(path)?.unwrap_or_else(|| id.to_string()))
    }
}

/// Apply similarity boosting based on project context.
///
/// - Same project: no change (identity)
/// - Different project: 0.875x penalty (12.5pp spread)
/// - Global or no context: no change
pub fn boost_similarity(
    base: f32,
    mem_project: Option<&str>,
    current_project: Option<&str>,
) -> f32 {
    match (mem_project, current_project) {
        (Some(m), Some(c)) if m == c => base, // Same project: no boost needed
        (Some(_), Some(_)) => base * 0.875,   // Different project: 12.5pp penalty
        _ => base,                            // Global or no context
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_scope_default_is_project() {
        // Fix #17: ListScope::default() should be Project, matching the tool schema default
        assert_eq!(ListScope::default(), ListScope::Project);
    }

    #[test]
    fn test_sha256_hex() {
        // sediment keyed its central project configs by this format
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn test_default_db_path_falls_back_to_sediment() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = tmp.path();
        std::fs::create_dir_all(home.join(".sediment/data")).unwrap();
        assert_eq!(default_db_path(home), home.join(".sediment/data"));

        std::fs::create_dir(home.join(".sand")).unwrap();
        assert_eq!(default_db_path(home), home.join(".sand/data"));
    }

    #[test]
    fn test_store_scope_default_is_project() {
        assert_eq!(StoreScope::default(), StoreScope::Project);
    }

    #[test]
    fn test_project_id_outside_git_is_stable_without_marker() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = tmp.path().join("access.db");
        let dir = tmp.path().join("p");
        std::fs::create_dir(&dir).unwrap();
        let first = resolve_project(&dir, &db).unwrap();
        assert_eq!(resolve_project(&dir, &db).unwrap(), first);
        assert!(!dir.join(".sand").exists() && !dir.join(".sediment").exists());
    }

    #[test]
    fn test_subdirectory_resolves_to_parent_project() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = tmp.path().join("access.db");
        let dir = tmp.path().join("p");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let parent = resolve_project(&dir, &db).unwrap();
        assert_eq!(resolve_project(&dir.join("sub"), &db).unwrap(), parent);
    }

    #[test]
    fn test_sediment_config_id_is_kept() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("p");
        std::fs::create_dir_all(dir.join(".sediment")).unwrap();
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        let config = format!(r#"{{"project_id": "{uuid}", "source": "uuid"}}"#);
        std::fs::write(dir.join(".sediment/config"), config).unwrap();
        let project = resolve_project(&dir, &tmp.path().join("access.db")).unwrap();
        assert_eq!(project.id, uuid);
    }

    #[test]
    fn test_boost_similarity() {
        assert!((boost_similarity(0.5, Some("p1"), Some("p1")) - 0.5).abs() < 0.001);
        assert!((boost_similarity(0.5, Some("p1"), Some("p2")) - 0.4375).abs() < 0.001);
        assert!((boost_similarity(0.5, None, Some("p1")) - 0.5).abs() < 0.001);
    }

    #[test]
    #[ignore] // requires git
    fn test_derive_git_root_commit_in_repo() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path();

        // git init + commit
        Command::new("git")
            .args(["init"])
            .current_dir(dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(dir)
            .output()
            .unwrap();

        let result = derive_git_root_commit(dir).unwrap();
        assert!(result.is_some(), "Should return root commit hash");
        let hash = result.unwrap();
        assert_eq!(hash.len(), 40, "SHA-1 hash should be 40 chars");
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()), "Should be hex");
    }

    #[test]
    #[ignore] // requires git
    fn test_derive_git_root_commit_no_commits() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path();

        Command::new("git")
            .args(["init"])
            .current_dir(dir)
            .output()
            .unwrap();

        let result = derive_git_root_commit(dir).unwrap();
        assert!(result.is_none(), "Repo with no commits should return None");
    }

    #[test]
    fn test_derive_git_root_commit_no_git() {
        let tmp = tempfile::TempDir::new().unwrap();
        let result = derive_git_root_commit(tmp.path()).unwrap();
        assert!(result.is_none(), "Non-git directory should return None");
    }

    fn git_repo(dir: &Path) {
        for args in [
            &["init"][..],
            &["config", "user.email", "test@test.com"],
            &["config", "user.name", "Test"],
            &["commit", "--allow-empty", "-m", "init"],
        ] {
            Command::new("git")
                .args(args)
                .current_dir(dir)
                .output()
                .unwrap();
        }
    }

    #[test]
    #[ignore] // requires git
    fn test_project_id_from_git_root_commit_stores_nothing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = tmp.path().join("access.db");
        let dir = tmp.path().join("repo");
        std::fs::create_dir(&dir).unwrap();
        git_repo(&dir);
        let project = resolve_project(&dir, &db).unwrap();
        assert_eq!(project.id, derive_git_root_commit(&dir).unwrap().unwrap());
        let rows: i64 = ProjectTable::open(&db)
            .unwrap()
            .conn
            .query_row("SELECT count(*) FROM projects", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 0);
    }

    #[test]
    #[ignore] // requires git
    fn test_project_id_migration_uuid_to_git() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = tmp.path().join("access.db");
        let dir = tmp.path().join("repo");
        std::fs::create_dir(&dir).unwrap();
        let old = resolve_project(&dir, &db).unwrap().id;
        git_repo(&dir);

        let project = resolve_project(&dir, &db).unwrap();
        assert_eq!(project.migrate_from.as_deref(), Some(old.as_str()));
        finish_project_migration(&db, &old, &project.id).unwrap();
        assert_eq!(resolve_project(&dir, &db).unwrap().migrate_from, None);
    }

    #[test]
    #[ignore] // requires git
    fn test_shallow_clone_falls_back_to_uuid() {
        let tmp = tempfile::TempDir::new().unwrap();
        let origin_dir = tmp.path().join("origin");
        let shallow_dir = tmp.path().join("shallow");
        std::fs::create_dir_all(&origin_dir).unwrap();

        // Create origin repo with a commit
        Command::new("git")
            .args(["init"])
            .current_dir(&origin_dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(&origin_dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&origin_dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(&origin_dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "--allow-empty", "-m", "second"])
            .current_dir(&origin_dir)
            .output()
            .unwrap();

        // Shallow clone (file:// protocol required for local shallow clones)
        let origin_url = format!("file://{}", origin_dir.display());
        Command::new("git")
            .args([
                "clone",
                "--depth=1",
                &origin_url,
                shallow_dir.to_str().unwrap(),
            ])
            .output()
            .unwrap();

        // derive_git_root_commit should return None for shallow clone
        let result = derive_git_root_commit(&shallow_dir).unwrap();
        assert!(result.is_none(), "Shallow clone should return None");
    }
}
