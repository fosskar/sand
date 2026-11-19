//! The backend on a database directory of this machine.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use serde_json::{Value, json};
use tokio::sync::Semaphore;

use super::{Backend, BackendError, Compacted, Recalled, StoreRequest, Stored, failed};
use crate::access::AccessTracker;
use crate::compaction::spawn_compaction;
use crate::consolidation::{ConsolidationQueue, run_consolidation_batch, spawn_consolidation};
use crate::db::{CompactConfig, DatabaseStats, is_valid_id, score_with_decay};
use crate::graph::GraphStore;
use crate::item::{Item, ItemFilters};
use crate::{Database, Embedder, ListScope, StoreScope};

/// The LanceDB directory and the `access.db` next to it.
pub struct LocalBackend {
    db_path: PathBuf,
    access_db_path: PathBuf,
    project_id: Option<String>,
    /// Loaded on first use, so commands that fail early or need no model skip the load.
    embedder: OnceLock<Arc<Embedder>>,
    background: Option<BackgroundWork>,
}

/// State of the MCP server's background work: consolidation, compaction, co-access, clustering.
struct BackgroundWork {
    embedder: Arc<Embedder>,
    /// Ensures only one consolidation task runs at a time
    consolidation_semaphore: Arc<Semaphore>,
    /// Ensures only one compaction task runs at a time
    compaction_semaphore: Arc<Semaphore>,
    /// Recall invocations; every 10th triggers clustering and queue cleanup
    recall_count: AtomicU64,
    /// Write operations (store/forget); every 50th triggers compaction
    write_count: AtomicU64,
}

impl LocalBackend {
    /// Open the store at `db_path`; `access.db` is its sibling.
    pub fn new(db_path: PathBuf, project_id: Option<String>) -> Self {
        let access_db_path = db_path.parent().unwrap_or(&db_path).join("access.db");
        Self {
            db_path,
            access_db_path,
            project_id,
            embedder: OnceLock::new(),
            background: None,
        }
    }

    /// Use `embedder` and run background work after store, recall and forget, as the MCP server does.
    pub fn with_background_work(self, embedder: Arc<Embedder>) -> Self {
        Self {
            embedder: OnceLock::from(embedder.clone()),
            background: Some(BackgroundWork {
                embedder,
                consolidation_semaphore: Arc::new(Semaphore::new(1)),
                compaction_semaphore: Arc::new(Semaphore::new(1)),
                recall_count: AtomicU64::new(0),
                write_count: AtomicU64::new(0),
            }),
            ..self
        }
    }

    fn embedder(&self) -> Result<Arc<Embedder>, BackendError> {
        if let Some(embedder) = self.embedder.get() {
            return Ok(embedder.clone());
        }
        let embedder = Arc::new(Embedder::new().map_err(failed("Failed to load embedder"))?);
        Ok(self.embedder.get_or_init(|| embedder).clone())
    }

    /// Open a fresh connection; the embedder is shared.
    async fn open_db(&self) -> Result<Database, BackendError> {
        let embedder = self.embedder()?;
        Database::open_with_embedder(&self.db_path, self.project_id.clone(), embedder)
            .await
            .map_err(failed("Failed to open database"))
    }

    fn open_graph(&self) -> Result<GraphStore, BackendError> {
        GraphStore::open(&self.access_db_path).map_err(failed("Failed to open graph store"))
    }

    /// Increment the write counter and spawn background compaction every 50 writes.
    fn maybe_spawn_compaction(&self) {
        let Some(work) = &self.background else {
            return;
        };
        let count = work.write_count.fetch_add(1, Ordering::AcqRel);
        if count % 50 == 49 {
            spawn_compaction(
                Arc::new(self.db_path.clone()),
                self.project_id.clone(),
                work.embedder.clone(),
                work.compaction_semaphore.clone(),
            );
        }
    }

    /// Process a batch of the consolidation queue in the background, unless one runs already.
    fn start_consolidation(&self, work: &BackgroundWork) {
        spawn_consolidation(
            Arc::new(self.db_path.clone()),
            Arc::new(self.access_db_path.clone()),
            self.project_id.clone(),
            work.embedder.clone(),
            work.consolidation_semaphore.clone(),
        );
    }

    /// Background work after a recall with results.
    fn spawn_after_recall(&self, work: &BackgroundWork, result_ids: Vec<String>) {
        self.start_consolidation(work);

        let access_db_path = self.access_db_path.clone();
        spawn_logged("co_access", async move {
            if let Ok(g) = GraphStore::open(&access_db_path) {
                if let Err(e) = g.record_co_access(&result_ids) {
                    tracing::warn!("record_co_access failed: {}", e);
                }
            } else {
                tracing::warn!("co_access: failed to open graph store");
            }
        });

        // Periodic maintenance: every 10th recall
        let run_count = work.recall_count.fetch_add(1, Ordering::AcqRel);
        if run_count % 10 == 9 {
            // Clustering
            let access_db_path = self.access_db_path.clone();
            spawn_logged("clustering", async move {
                if let Ok(g) = GraphStore::open(&access_db_path)
                    && let Ok(clusters) = g.detect_clusters()
                {
                    for (a, b, c) in &clusters {
                        let label = format!("cluster-{}", &a[..8.min(a.len())]);
                        if let Err(e) = g.add_related_edge(a, b, 0.8, &label) {
                            tracing::warn!("cluster add_related_edge failed: {}", e);
                        }
                        if let Err(e) = g.add_related_edge(b, c, 0.8, &label) {
                            tracing::warn!("cluster add_related_edge failed: {}", e);
                        }
                        if let Err(e) = g.add_related_edge(a, c, 0.8, &label) {
                            tracing::warn!("cluster add_related_edge failed: {}", e);
                        }
                    }
                    if !clusters.is_empty() {
                        tracing::info!("Detected {} clusters", clusters.len());
                    }
                }
            });

            // Consolidation queue cleanup: purge old processed entries
            let access_db_path2 = self.access_db_path.clone();
            spawn_logged("consolidation_cleanup", async move {
                if let Ok(q) = ConsolidationQueue::open(&access_db_path2)
                    && let Err(e) = q.cleanup_processed()
                {
                    tracing::warn!("consolidation queue cleanup failed: {}", e);
                }
            });
        }
    }
}

impl Backend for LocalBackend {
    fn project_id(&self) -> Option<&str> {
        self.project_id.as_deref()
    }

    async fn store(&self, request: StoreRequest) -> Result<Stored, BackendError> {
        let mut db = self.open_db().await?;
        let graph = self.open_graph()?;

        let mut item = Item::new(&request.content);

        #[cfg(feature = "bench")]
        if let Some(ts) = request.created_at {
            if let Some(dt) = chrono::DateTime::from_timestamp(ts, 0) {
                item = item.with_created_at(dt);
            }
        }

        if request.scope == StoreScope::Project
            && let Some(ref project_id) = self.project_id
        {
            item = item.with_project_id(project_id);
        }

        let store_result = db
            .store_item(item)
            .await
            .map_err(failed("Failed to store item"))?;
        let new_id = store_result.id.clone();

        // Create graph node
        let now = chrono::Utc::now().timestamp();
        if let Err(e) = graph.add_node(&new_id, self.project_id.as_deref(), now) {
            tracing::warn!("graph add_node failed: {}", e);
        }

        // Enqueue consolidation candidates from conflicts
        if !store_result.potential_conflicts.is_empty()
            && let Ok(queue) = ConsolidationQueue::open(&self.access_db_path)
        {
            for conflict in &store_result.potential_conflicts {
                if let Err(e) = queue.enqueue(&new_id, &conflict.id, conflict.similarity as f64) {
                    tracing::warn!("enqueue consolidation failed: {}", e);
                }
            }
        }

        // Handle replace: delete old item, preserve graph lineage
        let mut replaced_id = None;
        if let Some(old_id) = request.replace_id {
            if !is_valid_id(&old_id) {
                tracing::warn!("replace_id is not a valid ID: {}", old_id);
            } else {
                if let Err(e) = graph.add_supersedes_edge(&new_id, &old_id) {
                    tracing::warn!("replace: add_supersedes_edge failed: {}", e);
                }
                if let Err(e) = graph.transfer_edges(&old_id, &new_id) {
                    tracing::warn!("replace: transfer_edges failed: {}", e);
                }

                // a replacement confirms the note, which raises its trust bonus in recall
                if let Ok(tracker) = AccessTracker::open(&self.access_db_path) {
                    let created_at = chrono::Utc::now().timestamp();
                    if let Err(e) = tracker.record_validation(&new_id, created_at) {
                        tracing::warn!("replace: record_validation failed: {}", e);
                    }
                }

                let deleted = match db.delete_item(&old_id).await {
                    Ok(true) => true,
                    Ok(false) => {
                        tracing::warn!("replace: old item not found: {}", old_id);
                        false
                    }
                    Err(e) => {
                        tracing::warn!("replace: delete_item failed: {}", e);
                        false
                    }
                };
                if let Err(e) = graph.remove_node(&old_id) {
                    tracing::warn!("replace: remove_node failed: {}", e);
                }
                if deleted {
                    replaced_id = Some(old_id);
                }
            }
        }

        // Periodic compaction: every 50 write operations
        self.maybe_spawn_compaction();

        // merge or link the near-duplicates the store queued
        match &self.background {
            Some(work) => self.start_consolidation(work),
            // a spawned task would not survive the CLI process; one batch keeps the command quick
            None if !store_result.potential_conflicts.is_empty() => {
                if let Err(e) = run_consolidation_batch(
                    &self.db_path,
                    &self.access_db_path,
                    self.project_id.clone(),
                    self.embedder()?,
                )
                .await
                {
                    tracing::warn!("consolidation after store failed: {}", e);
                }
            }
            None => {}
        }

        Ok(Stored {
            id: new_id,
            potential_conflicts: store_result.potential_conflicts,
            replaced_id,
        })
    }

    async fn recall(&self, query: &str, limit: usize) -> Result<Recalled, BackendError> {
        let mut db = self.open_db().await?;
        let graph = self.open_graph()?;
        let tracker = AccessTracker::open(&self.access_db_path)
            .map_err(failed("Failed to open access tracker"))?;

        let config = RecallConfig {
            enable_background_tasks: self.background.is_some(),
            ..Default::default()
        };
        let result = recall_pipeline(
            &mut db,
            &tracker,
            &graph,
            query,
            limit,
            ItemFilters::new(),
            &config,
        )
        .await
        .map_err(failed("Search failed"))?;

        if result.results.is_empty() {
            return Ok(Recalled {
                result,
                related_ids: HashMap::new(),
            });
        }

        // Batch-fetch neighbors for all result IDs
        let result_ids: Vec<&str> = result.results.iter().map(|r| r.id.as_str()).collect();
        let related_ids = graph
            .get_neighbors_mapped(&result_ids, 0.5)
            .unwrap_or_default();

        if let Some(work) = &self.background {
            let result_ids = result.results.iter().map(|r| r.id.clone()).collect();
            self.spawn_after_recall(work, result_ids);
        }

        Ok(Recalled {
            result,
            related_ids,
        })
    }

    async fn list(&self, limit: usize, scope: ListScope) -> Result<Vec<Item>, BackendError> {
        let mut db = self.open_db().await?;
        db.list_items(Some(limit), scope)
            .await
            .map_err(failed("Failed to list items"))
    }

    async fn forget(&self, id: &str) -> Result<(), BackendError> {
        let db = self.open_db().await?;
        let graph = self.open_graph()?;

        // Access control: verify the item belongs to the current project (or is global)
        if let Some(ref current_pid) = self.project_id {
            match db.get_item(id).await {
                Ok(Some(item)) => {
                    if let Some(ref item_pid) = item.project_id
                        && item_pid != current_pid
                    {
                        return Err(BackendError::OtherProject(id.to_string()));
                    }
                }
                Ok(None) => return Err(BackendError::NotFound(id.to_string())),
                Err(e) => return Err(failed("Failed to look up item")(e)),
            }
        }

        match db.delete_item(id).await {
            Ok(true) => {
                if let Err(e) = graph.remove_node(id) {
                    tracing::warn!("remove_node failed: {}", e);
                }
                // Periodic compaction: every 50 write operations
                self.maybe_spawn_compaction();
                Ok(())
            }
            Ok(false) => Err(BackendError::NotFound(id.to_string())),
            Err(e) => Err(failed("Failed to delete item")(e)),
        }
    }

    async fn stats(&self) -> Result<DatabaseStats, BackendError> {
        let db = self.open_db().await?;
        db.stats()
            .await
            .map_err(failed("Failed to read statistics"))
    }

    async fn compact(&self, force: bool, no_prune: bool) -> Result<Compacted, BackendError> {
        let size_before = crate::db::dir_size(&self.db_path);

        let config = CompactConfig {
            prune_older_than: if no_prune {
                None
            } else {
                Some(chrono::Duration::zero())
            },
            delete_unverified: force,
            num_threads: None, // use all CPUs
            skip_prune: no_prune,
        };

        let db = self.open_db().await?;
        let stats = db
            .optimize_tables(&config)
            .await
            .map_err(failed("Failed to compact"))?;

        Ok(Compacted {
            stats,
            size_before,
            size_after: crate::db::dir_size(&self.db_path),
        })
    }

    async fn migrate_project(&self, old_id: &str, new_id: &str) {
        if let Err(e) = crate::db::migrate_project_id(&self.db_path, old_id, new_id).await {
            tracing::warn!("Failed to migrate LanceDB items: {}", e);
        }

        if let Ok(graph) = GraphStore::open(&self.access_db_path)
            && let Err(e) = graph.migrate_project_id(old_id, new_id)
        {
            tracing::warn!("Failed to migrate graph nodes: {}", e);
        }
    }

    async fn finish(&self) {
        if let Some(work) = &self.background {
            let _ = work.consolidation_semaphore.acquire().await;
            let _ = work.compaction_semaphore.acquire().await;
        }
    }
}

/// Spawn a background task with panic logging. If the task panics, the panic
/// is caught and logged as an error instead of silently disappearing.
fn spawn_logged(name: &'static str, fut: impl std::future::Future<Output = ()> + Send + 'static) {
    tokio::spawn(async move {
        let result = tokio::task::spawn(fut).await;
        if let Err(e) = result {
            tracing::error!("Background task '{}' panicked: {:?}", name, e);
        }
    });
}

// ========== Recall Configuration ==========

/// Controls which graph and scoring features are enabled during recall.
/// Used by benchmarks to measure the impact of individual features.
pub struct RecallConfig {
    pub enable_graph_backfill: bool,
    pub enable_graph_expansion: bool,
    pub enable_co_access: bool,
    pub enable_decay_scoring: bool,
    pub enable_background_tasks: bool,
}

impl Default for RecallConfig {
    fn default() -> Self {
        Self {
            enable_graph_backfill: true,
            enable_graph_expansion: true,
            enable_co_access: true,
            enable_decay_scoring: true,
            enable_background_tasks: true,
        }
    }
}

/// Result of a recall pipeline execution (for benchmark consumption).
pub struct RecallResult {
    pub results: Vec<crate::item::SearchResult>,
    pub graph_expanded: Vec<Value>,
    pub suggested: Vec<Value>,
    /// Raw (pre-decay/trust) similarity scores, keyed by item ID
    pub raw_similarities: std::collections::HashMap<String, f32>,
}

/// Core recall pipeline, extracted for benchmarking.
///
/// Performs: vector search, optional decay scoring, optional graph backfill,
/// optional 1-hop graph expansion, and optional co-access suggestions.
pub async fn recall_pipeline(
    db: &mut Database,
    tracker: &AccessTracker,
    graph: &GraphStore,
    query: &str,
    limit: usize,
    filters: ItemFilters,
    config: &RecallConfig,
) -> crate::Result<RecallResult> {
    let mut results = db.search_items(query, limit, filters).await?;

    if results.is_empty() {
        return Ok(RecallResult {
            results: Vec::new(),
            graph_expanded: Vec::new(),
            suggested: Vec::new(),
            raw_similarities: std::collections::HashMap::new(),
        });
    }

    // Lazy graph backfill (uses project_id from SearchResult, no extra queries)
    if config.enable_graph_backfill {
        for result in &results {
            if let Err(e) = graph.add_node(
                &result.id,
                result.project_id.as_deref(),
                result.created_at.timestamp(),
            ) {
                tracing::warn!("graph backfill failed: {}", e);
            }
        }
    }

    // Decay scoring — preserve raw similarity for transparency
    let mut raw_similarities: std::collections::HashMap<String, f32> =
        std::collections::HashMap::new();
    if config.enable_decay_scoring {
        let item_ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
        let decay_data = tracker.get_decay_data(&item_ids).unwrap_or_default();
        let edge_counts = graph.get_edge_counts(&item_ids).unwrap_or_default();
        let now = chrono::Utc::now().timestamp();

        for result in &mut results {
            raw_similarities.insert(result.id.clone(), result.similarity);

            let created_at = result.created_at.timestamp();
            let (access_count, last_accessed, validation_count) = match decay_data.get(&result.id) {
                Some(data) => (
                    data.access_count,
                    Some(data.last_accessed_at),
                    data.validation_count,
                ),
                None => (0, None, 0),
            };

            let base_score = score_with_decay(
                result.similarity,
                now,
                created_at,
                access_count,
                last_accessed,
            );

            let edge_count = edge_counts.get(&result.id).copied().unwrap_or(0);
            let trust_bonus = 1.0
                + 0.05 * (1.0 + validation_count as f64).ln() as f32
                + 0.005 * edge_count as f32;

            result.similarity = (base_score * trust_bonus).min(1.0);
        }

        results.sort_by(|a, b| {
            b.similarity
                .partial_cmp(&a.similarity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    // Record access
    for result in &results {
        let created_at = result.created_at.timestamp();
        if let Err(e) = tracker.record_access(&result.id, created_at) {
            tracing::warn!("record_access failed: {}", e);
        }
    }

    // Graph expansion
    let existing_ids: std::collections::HashSet<String> =
        results.iter().map(|r| r.id.clone()).collect();

    let mut graph_expanded = Vec::new();
    if config.enable_graph_expansion {
        let top_ids: Vec<&str> = results.iter().take(5).map(|r| r.id.as_str()).collect();
        if let Ok(neighbors) = graph.get_neighbors(&top_ids, 0.5) {
            // Collect neighbor IDs not already in results, then batch fetch
            let neighbor_info: Vec<(String, String)> = neighbors
                .into_iter()
                .filter(|(id, _, _)| !existing_ids.contains(id))
                .map(|(id, rel_type, _)| (id, rel_type))
                .collect();

            let neighbor_ids: Vec<&str> = neighbor_info.iter().map(|(id, _)| id.as_str()).collect();
            if let Ok(items) = db.get_items_batch(&neighbor_ids).await {
                let item_map: std::collections::HashMap<&str, &Item> =
                    items.iter().map(|item| (item.id.as_str(), item)).collect();

                for (neighbor_id, rel_type) in &neighbor_info {
                    if let Some(item) = item_map.get(neighbor_id.as_str()) {
                        let sr = crate::item::SearchResult::from_item(item, 0.05);
                        let mut entry = json!({
                            "id": sr.id,
                            "similarity": "graph",
                            "created": sr.created_at.to_rfc3339(),
                            "graph_expanded": true,
                            "rel_type": rel_type,
                        });
                        // Only include content for same-project or global items
                        let same_project = match (db.project_id(), item.project_id.as_deref()) {
                            (Some(current), Some(item_pid)) => current == item_pid,
                            (_, None) => true,
                            _ => false,
                        };
                        if same_project {
                            entry["content"] = json!(sr.content);
                        } else {
                            entry["cross_project"] = json!(true);
                        }
                        graph_expanded.push(entry);
                    }
                }
            }
        }
    }

    // Co-access suggestions (batch fetch)
    let mut suggested = Vec::new();
    if config.enable_co_access {
        let top3_ids: Vec<&str> = results.iter().take(3).map(|r| r.id.as_str()).collect();
        if let Ok(co_accessed) = graph.get_co_accessed(&top3_ids, 3) {
            let co_info: Vec<(String, i64)> = co_accessed
                .into_iter()
                .filter(|(id, _)| !existing_ids.contains(id))
                .collect();

            let co_ids: Vec<&str> = co_info.iter().map(|(id, _)| id.as_str()).collect();
            if let Ok(items) = db.get_items_batch(&co_ids).await {
                let item_map: std::collections::HashMap<&str, &Item> =
                    items.iter().map(|item| (item.id.as_str(), item)).collect();

                for (co_id, co_count) in &co_info {
                    if let Some(item) = item_map.get(co_id.as_str()) {
                        let same_project = match (db.project_id(), item.project_id.as_deref()) {
                            (Some(current), Some(item_pid)) => current == item_pid,
                            (_, None) => true,
                            _ => false,
                        };
                        let mut entry = json!({
                            "id": item.id,
                            "reason": format!("frequently recalled with result (co-accessed {} times)", co_count),
                        });
                        if same_project {
                            entry["content"] = json!(truncate(&item.content, 100));
                        } else {
                            entry["cross_project"] = json!(true);
                        }
                        suggested.push(entry);
                    }
                }
            }
        }
    }

    Ok(RecallResult {
        results,
        graph_expanded,
        suggested,
        raw_similarities,
    })
}

pub(crate) fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else if max_len <= 3 {
        // Not enough room for "..." suffix; just take max_len chars
        s.chars().take(max_len).collect()
    } else {
        let cut = s
            .char_indices()
            .nth(max_len - 3)
            .map(|(i, _)| i)
            .unwrap_or(s.len());
        format!("{}...", &s[..cut])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_small_max_len() {
        // Bug #25: truncate should not panic when max_len < 3
        assert_eq!(truncate("hello", 0), "");
        assert_eq!(truncate("hello", 1), "h");
        assert_eq!(truncate("hello", 2), "he");
        assert_eq!(truncate("hello", 3), "hel");
        assert_eq!(truncate("hi", 3), "hi"); // shorter than max, no truncation
        assert_eq!(truncate("hello", 5), "hello");
        assert_eq!(truncate("hello!", 5), "he...");
    }

    #[test]
    fn test_truncate_unicode() {
        assert_eq!(truncate("héllo wörld", 5), "hé...");
        assert_eq!(truncate("日本語テスト", 4), "日...");
    }
}
