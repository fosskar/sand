//! The operations the CLI and the MCP server run against a memory store.

use std::collections::HashMap;

use crate::db::{CompactStats, DatabaseStats};
use crate::item::{ConflictInfo, Item};
use crate::{ListScope, StoreScope};

mod local;

pub(crate) use local::truncate;
pub use local::{LocalBackend, RecallConfig, RecallResult, recall_pipeline};

/// A memory store as seen from one project, or from no project (global context).
// the front ends drive one backend from one task, so the futures need no `Send` bound
#[allow(async_fn_in_trait)]
pub trait Backend {
    /// Project the operations act for; `None` is the global context.
    fn project_id(&self) -> Option<&str>;

    async fn store(&self, request: StoreRequest) -> Result<Stored, BackendError>;

    /// Search by semantic similarity, with decay scoring, trust weighting and graph expansion.
    async fn recall(&self, query: &str, limit: usize) -> Result<Recalled, BackendError>;

    async fn list(&self, limit: usize, scope: ListScope) -> Result<Vec<Item>, BackendError>;

    /// Delete an item of this project or a global item.
    async fn forget(&self, id: &str) -> Result<(), BackendError>;

    async fn stats(&self) -> Result<DatabaseStats, BackendError>;

    /// Compact the storage; `force` also deletes unverified files, `no_prune` keeps old versions.
    async fn compact(&self, force: bool, no_prune: bool) -> Result<Compacted, BackendError>;

    /// Drain the consolidation queue until it is empty or `max_batches` batches ran.
    async fn consolidate(&self, max_batches: usize) -> Result<Consolidated, BackendError>;

    /// Move the items and graph nodes of project `old_id` to `new_id`; failures are logged.
    async fn migrate_project(&self, old_id: &str, new_id: &str);

    /// Wait for running background work before the process exits.
    async fn finish(&self);
}

/// Content to store.
#[derive(Debug)]
pub struct StoreRequest {
    pub content: String,
    pub scope: StoreScope,
    /// Item the new one replaces: its graph edges move to the new item, then it is deleted.
    pub replace_id: Option<String>,
    /// Creation time in unix seconds; honoured only in builds with the `bench` feature.
    pub created_at: Option<i64>,
}

#[derive(Debug)]
pub struct Stored {
    pub id: String,
    pub potential_conflicts: Vec<ConflictInfo>,
    /// Set when `replace_id` named an item that was deleted.
    pub replaced_id: Option<String>,
}

pub struct Recalled {
    pub result: RecallResult,
    /// Graph neighbours of each result, keyed by result id.
    pub related_ids: HashMap<String, Vec<String>>,
}

#[derive(Debug)]
pub struct Compacted {
    pub stats: CompactStats,
    pub size_before: u64,
    pub size_after: u64,
}

#[derive(Debug)]
pub struct Consolidated {
    pub batches: usize,
    /// Candidates still pending.
    pub remaining: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    /// `context` names the failed step without internals; `error` has the details.
    #[error("{context}: {error}")]
    Failed {
        context: &'static str,
        error: String,
    },
    #[error("Item not found: {0}")]
    NotFound(String),
    #[error("Cannot delete item {0} from a different project")]
    OtherProject(String),
}

/// Map an error to `BackendError::Failed` with `context`.
fn failed<E: std::fmt::Display>(context: &'static str) -> impl FnOnce(E) -> BackendError {
    move |error| BackendError::Failed {
        context,
        error: error.to_string(),
    }
}
