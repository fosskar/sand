//! The backend on a database directory of this machine.

use serde_json::{Value, json};

use crate::Database;
use crate::access::AccessTracker;
use crate::db::score_with_decay;
use crate::graph::GraphStore;
use crate::item::{Item, ItemFilters};

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
) -> std::result::Result<RecallResult, String> {
    let mut results = db
        .search_items(query, limit, filters)
        .await
        .map_err(|e| format!("Search failed: {}", e))?;

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
