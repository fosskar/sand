//! Graph store using SQLite for relationship tracking between memories.
//!
//! Provides a graph layer alongside LanceDB for tracking
//! relationships like RELATED, SUPERSEDES, and CO_ACCESSED between items.

use std::path::Path;

use rusqlite::{Connection, params};
use tracing::debug;

use crate::error::{Result, SandError};

/// SQLite-backed graph store for memory relationships.
pub struct GraphStore {
    conn: Connection,
}

impl GraphStore {
    /// Open or create the graph store using the given SQLite database path.
    /// Shares the same file as access.db.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .map_err(|e| SandError::Database(format!("Failed to open graph database: {}", e)))?;

        if let Err(e) = conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;") {
            tracing::warn!("Failed to set SQLite PRAGMAs (graph): {}", e);
        }

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS graph_nodes (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS graph_edges (
                from_id TEXT NOT NULL,
                to_id TEXT NOT NULL,
                edge_type TEXT NOT NULL,
                strength REAL NOT NULL DEFAULT 0.0,
                rel_type TEXT NOT NULL DEFAULT '',
                count INTEGER NOT NULL DEFAULT 0,
                last_at INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                UNIQUE(from_id, to_id, edge_type)
            );

            CREATE INDEX IF NOT EXISTS idx_edges_from ON graph_edges(from_id, edge_type);
            CREATE INDEX IF NOT EXISTS idx_edges_to ON graph_edges(to_id, edge_type);",
        )
        .map_err(|e| SandError::Database(format!("Failed to create graph tables: {}", e)))?;

        Ok(Self { conn })
    }

    /// Add a Memory node to the graph.
    pub fn add_node(&self, id: &str, project_id: Option<&str>, created_at: i64) -> Result<()> {
        let pid = project_id.unwrap_or("");

        self.conn
            .execute(
                "INSERT OR IGNORE INTO graph_nodes (id, project_id, created_at) VALUES (?1, ?2, ?3)",
                params![id, pid, created_at],
            )
            .map_err(|e| SandError::Database(format!("Failed to add node: {}", e)))?;

        debug!("Added graph node: {}", id);
        Ok(())
    }

    /// Remove a Memory node and all its edges from the graph.
    pub fn remove_node(&self, id: &str) -> Result<()> {
        // Preserve incoming SUPERSEDES edges (where this node is the to_id)
        // so that lineage/provenance chains remain intact after node removal.
        self.conn
            .execute(
                "DELETE FROM graph_edges WHERE from_id = ?1 OR (to_id = ?1 AND edge_type != 'supersedes')",
                params![id],
            )
            .map_err(|e| SandError::Database(format!("Failed to remove edges: {}", e)))?;

        self.conn
            .execute("DELETE FROM graph_nodes WHERE id = ?1", params![id])
            .map_err(|e| SandError::Database(format!("Failed to remove node: {}", e)))?;

        debug!("Removed graph node: {}", id);
        Ok(())
    }

    /// Add a RELATED edge between two Memory nodes.
    pub fn add_related_edge(
        &self,
        from_id: &str,
        to_id: &str,
        strength: f64,
        rel_type: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp();

        self.conn
            .execute(
                "INSERT OR IGNORE INTO graph_edges (from_id, to_id, edge_type, strength, rel_type, created_at)
                 VALUES (?1, ?2, 'related', ?3, ?4, ?5)",
                params![from_id, to_id, strength, rel_type, now],
            )
            .map_err(|e| SandError::Database(format!("Failed to add related edge: {}", e)))?;

        debug!(
            "Added RELATED edge: {} -> {} ({})",
            from_id, to_id, rel_type
        );
        Ok(())
    }

    /// Add a SUPERSEDES edge from new_id to old_id.
    pub fn add_supersedes_edge(&self, new_id: &str, old_id: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp();

        self.conn
            .execute(
                "INSERT OR IGNORE INTO graph_edges (from_id, to_id, edge_type, strength, created_at)
                 VALUES (?1, ?2, 'supersedes', 1.0, ?3)",
                params![new_id, old_id, now],
            )
            .map_err(|e| SandError::Database(format!("Failed to add supersedes edge: {}", e)))?;

        debug!("Added SUPERSEDES edge: {} -> {}", new_id, old_id);
        Ok(())
    }

    /// Get 1-hop neighbors of the given item IDs via RELATED or SUPERSEDES edges.
    /// Returns (neighbor_id, rel_type, strength) tuples.
    ///
    /// Note on parameter binding: SQLite reuses the same positional parameters (?1..?N)
    /// across all three IN clauses and the CASE expression. This is correct because
    /// SQLite binds by position, so the same parameter set is applied to each reference.
    pub fn get_neighbors(
        &self,
        ids: &[&str],
        min_strength: f64,
    ) -> Result<Vec<(String, String, f64)>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{}", i)).collect();
        let ph = placeholders.join(",");
        let strength_idx = ids.len() + 1;

        let sql = format!(
            "SELECT
                CASE WHEN from_id IN ({ph}) THEN to_id ELSE from_id END AS neighbor,
                CASE WHEN edge_type = 'related' THEN rel_type ELSE 'supersedes' END AS rtype,
                strength
             FROM graph_edges
             WHERE (from_id IN ({ph}) OR to_id IN ({ph}))
               AND edge_type IN ('related', 'supersedes')
               AND strength >= ?{strength_idx}
             LIMIT 100"
        );

        let mut stmt = self.conn.prepare(&sql).map_err(|e| {
            SandError::Database(format!("Failed to prepare neighbors query: {}", e))
        })?;

        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        for id in ids {
            param_values.push(Box::new(id.to_string()));
        }
        param_values.push(Box::new(min_strength));

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|b| b.as_ref()).collect();

        let rows = stmt
            .query_map(params_ref.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, f64>(2)?,
                ))
            })
            .map_err(|e| SandError::Database(format!("Failed to query neighbors: {}", e)))?;

        // Filter out input IDs from results so we never return a query ID as its own neighbor
        let input_set: std::collections::HashSet<&str> = ids.iter().copied().collect();
        let mut results = Vec::new();
        for row in rows {
            let r =
                row.map_err(|e| SandError::Database(format!("Failed to read neighbor: {}", e)))?;
            if !input_set.contains(r.0.as_str()) {
                results.push(r);
            }
        }

        Ok(results)
    }

    /// Record co-access between pairs of item IDs.
    /// Creates or increments CO_ACCESSED edges.
    pub fn record_co_access(&self, item_ids: &[String]) -> Result<()> {
        if item_ids.len() < 2 {
            return Ok(());
        }

        // Performance optimization: limit co-access recording to the top 3 items
        // by position to bound the O(n^2) pair generation. Items beyond position 3
        // in recall results won't have co-access edges recorded.
        let item_ids = if item_ids.len() > 3 {
            &item_ids[..3]
        } else {
            item_ids
        };

        let now = chrono::Utc::now().timestamp();

        for i in 0..item_ids.len() {
            for j in (i + 1)..item_ids.len() {
                // Normalize edge direction: smaller ID always goes first to prevent
                // duplicate edges (A,B) and (B,A) from accumulating separately.
                let (a, b) = if item_ids[i] <= item_ids[j] {
                    (&item_ids[i], &item_ids[j])
                } else {
                    (&item_ids[j], &item_ids[i])
                };

                self.conn
                    .execute(
                        "INSERT INTO graph_edges (from_id, to_id, edge_type, count, last_at, created_at)
                         VALUES (?1, ?2, 'co_accessed', 1, ?3, ?3)
                         ON CONFLICT(from_id, to_id, edge_type)
                         DO UPDATE SET count = count + 1, last_at = ?3",
                        params![a, b, now],
                    )
                    .map_err(|e| {
                        SandError::Database(format!("Failed to record co-access: {}", e))
                    })?;
            }
        }

        Ok(())
    }

    /// Get items that are frequently co-accessed with the given IDs.
    /// Returns (neighbor_id, co_access_count) tuples.
    pub fn get_co_accessed(&self, ids: &[&str], min_count: i64) -> Result<Vec<(String, i64)>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{}", i)).collect();
        let ph = placeholders.join(",");
        let min_idx = ids.len() + 1;

        let sql = format!(
            "SELECT
                CASE WHEN from_id IN ({ph}) THEN to_id ELSE from_id END AS neighbor,
                count
             FROM graph_edges
             WHERE (from_id IN ({ph}) OR to_id IN ({ph}))
               AND edge_type = 'co_accessed'
               AND count >= ?{min_idx}
             ORDER BY count DESC
             LIMIT 50"
        );

        let mut stmt = self.conn.prepare(&sql).map_err(|e| {
            SandError::Database(format!("Failed to prepare co-access query: {}", e))
        })?;

        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        for id in ids {
            param_values.push(Box::new(id.to_string()));
        }
        param_values.push(Box::new(min_count));

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|b| b.as_ref()).collect();

        let rows = stmt
            .query_map(params_ref.as_slice(), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| SandError::Database(format!("Failed to query co-access: {}", e)))?;

        let mut results = Vec::new();
        for row in rows {
            let r =
                row.map_err(|e| SandError::Database(format!("Failed to read co-access: {}", e)))?;
            results.push(r);
        }

        // Deduplicate by target_id, keeping highest count
        results.sort_by_key(|a| std::cmp::Reverse(a.1));
        let mut seen = std::collections::HashSet::new();
        results.retain(|(id, _)| seen.insert(id.clone()));

        Ok(results)
    }

    /// Transfer all edges from one node to another (used during consolidation merge).
    pub fn transfer_edges(&self, from_id: &str, to_id: &str) -> Result<()> {
        // Get all RELATED edges connected to from_id (excluding edges to to_id)
        let mut stmt = self
            .conn
            .prepare(
                "SELECT from_id, to_id, strength, rel_type, created_at
             FROM graph_edges
             WHERE (from_id = ?1 OR to_id = ?1)
               AND edge_type = 'related'
               AND from_id != ?2 AND to_id != ?2",
            )
            .map_err(|e| SandError::Database(format!("Failed to prepare transfer query: {}", e)))?;

        let edges: Vec<(String, f64, String, i64)> = stmt
            .query_map(params![from_id, to_id], |row| {
                let fid: String = row.get(0)?;
                let tid: String = row.get(1)?;
                let neighbor = if fid == from_id { tid } else { fid };
                Ok((neighbor, row.get(2)?, row.get(3)?, row.get(4)?))
            })
            .map_err(|e| SandError::Database(format!("Failed to query edges for transfer: {}", e)))?
            .filter_map(|r| match r {
                Ok(v) => Some(v),
                Err(e) => {
                    tracing::warn!("transfer_edges: failed to read row: {}", e);
                    None
                }
            })
            .collect();

        // Create edges on the new node
        for (neighbor, strength, rel_type, _) in &edges {
            if let Err(e) = self.add_related_edge(to_id, neighbor, *strength, rel_type) {
                tracing::warn!("transfer edge to {} failed: {}", neighbor, e);
            }
        }

        Ok(())
    }

    /// Detect triangles of RELATED items (for clustering).
    /// Returns sets of 3 item IDs that form triangles.
    pub fn detect_clusters(&self) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self
            .conn
            .prepare(
                "WITH biedges AS (
                SELECT from_id AS a, to_id AS b FROM graph_edges WHERE edge_type = 'related'
                UNION ALL
                SELECT to_id AS a, from_id AS b FROM graph_edges WHERE edge_type = 'related'
            )
            SELECT DISTINCT e1.a, e1.b, e2.b
            FROM biedges e1
            JOIN biedges e2 ON e1.b = e2.a
            JOIN biedges e3 ON e2.b = e3.a AND e3.b = e1.a
            WHERE e1.a < e1.b AND e1.b < e2.b
            LIMIT 50",
            )
            .map_err(|e| SandError::Database(format!("Failed to detect clusters: {}", e)))?;

        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| SandError::Database(format!("Failed to read clusters: {}", e)))?;

        let mut clusters = Vec::new();
        for r in rows.flatten() {
            clusters.push(r);
        }

        Ok(clusters)
    }

    /// Get 1-hop neighbors grouped by source ID.
    ///
    /// For each input ID, returns a list of neighbor IDs from RELATED or SUPERSEDES edges.
    /// Input IDs are excluded from results.
    pub fn get_neighbors_mapped(
        &self,
        ids: &[&str],
        min_strength: f64,
    ) -> Result<std::collections::HashMap<String, Vec<String>>> {
        if ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }

        let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{}", i)).collect();
        let ph = placeholders.join(",");
        let strength_idx = ids.len() + 1;

        let sql = format!(
            "SELECT
                CASE WHEN from_id IN ({ph}) THEN from_id ELSE to_id END AS source,
                CASE WHEN from_id IN ({ph}) THEN to_id ELSE from_id END AS neighbor,
                strength
             FROM graph_edges
             WHERE (from_id IN ({ph}) OR to_id IN ({ph}))
               AND edge_type IN ('related', 'supersedes')
               AND strength >= ?{strength_idx}
             LIMIT 500"
        );

        let mut stmt = self.conn.prepare(&sql).map_err(|e| {
            SandError::Database(format!("Failed to prepare neighbors_mapped query: {}", e))
        })?;

        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        for id in ids {
            param_values.push(Box::new(id.to_string()));
        }
        param_values.push(Box::new(min_strength));

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|b| b.as_ref()).collect();

        let rows = stmt
            .query_map(params_ref.as_slice(), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| SandError::Database(format!("Failed to query neighbors_mapped: {}", e)))?;

        let input_set: std::collections::HashSet<&str> = ids.iter().copied().collect();
        let mut map: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();

        for row in rows {
            let (source, neighbor) = row.map_err(|e| {
                SandError::Database(format!("Failed to read neighbor_mapped: {}", e))
            })?;
            if !input_set.contains(neighbor.as_str()) {
                map.entry(source).or_default().push(neighbor);
            }
        }

        Ok(map)
    }

    /// Migrate all graph nodes from one project ID to another.
    ///
    /// Used when a project's ID changes (e.g., UUID→git root commit hash).
    pub fn migrate_project_id(&self, old_id: &str, new_id: &str) -> Result<usize> {
        let updated = self
            .conn
            .execute(
                "UPDATE graph_nodes SET project_id = ?1 WHERE project_id = ?2",
                params![new_id, old_id],
            )
            .map_err(|e| SandError::Database(format!("Failed to migrate graph nodes: {}", e)))?;

        if updated > 0 {
            debug!(
                "Migrated {} graph nodes from project {} to {}",
                updated, old_id, new_id
            );
        }
        Ok(updated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn open_test_graph() -> GraphStore {
        let tmp = NamedTempFile::new().unwrap();
        GraphStore::open(tmp.path()).unwrap()
    }

    #[test]
    fn test_get_neighbors_excludes_input_ids() {
        // Fix #7: get_neighbors should never return an input ID as a neighbor
        let graph = open_test_graph();
        let now = chrono::Utc::now().timestamp();
        graph.add_node("A", Some("proj"), now).unwrap();
        graph.add_node("B", Some("proj"), now).unwrap();
        graph.add_node("C", Some("proj"), now).unwrap();

        // Create edges A-B, B-C
        graph.add_related_edge("A", "B", 0.9, "test").unwrap();
        graph.add_related_edge("B", "C", 0.9, "test").unwrap();

        // Query neighbors for [A, B] — should only return C, not A or B
        let neighbors = graph.get_neighbors(&["A", "B"], 0.0).unwrap();
        let neighbor_ids: Vec<&str> = neighbors.iter().map(|(id, _, _)| id.as_str()).collect();
        assert!(neighbor_ids.contains(&"C"));
        assert!(!neighbor_ids.contains(&"A"));
        assert!(!neighbor_ids.contains(&"B"));
    }

    #[test]
    fn test_co_access_normalized_direction() {
        // Fix #8: co-access edges should be normalized so (A,B) and (B,A) don't create duplicates
        let graph = open_test_graph();
        let now = chrono::Utc::now().timestamp();
        graph.add_node("Z", Some("proj"), now).unwrap();
        graph.add_node("A", Some("proj"), now).unwrap();

        // Record co-access with Z before A (Z > A lexicographically)
        graph
            .record_co_access(&["Z".to_string(), "A".to_string()])
            .unwrap();
        // Record again with reversed order
        graph
            .record_co_access(&["A".to_string(), "Z".to_string()])
            .unwrap();

        // Should only have 1 edge with count=2 (not 2 edges with count=1 each)
        let count: i64 = graph
            .conn
            .query_row(
                "SELECT COUNT(*) FROM graph_edges WHERE edge_type = 'co_accessed'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "Should have exactly 1 co-access edge");

        let edge_count: i64 = graph
            .conn
            .query_row(
                "SELECT count FROM graph_edges WHERE edge_type = 'co_accessed'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(edge_count, 2, "Edge count should be 2 (incremented twice)");
    }

    #[test]
    fn test_transfer_edges_preserves_relationships() {
        // Fix #6: transfer_edges should move edges from old node to new node
        let graph = open_test_graph();
        let now = chrono::Utc::now().timestamp();
        graph.add_node("old", Some("proj"), now).unwrap();
        graph.add_node("new", Some("proj"), now).unwrap();
        graph.add_node("friend", Some("proj"), now).unwrap();

        graph
            .add_related_edge("old", "friend", 0.9, "test")
            .unwrap();

        // Transfer edges from old to new
        graph.transfer_edges("old", "new").unwrap();

        // New node should now have edge to friend
        let neighbors = graph.get_neighbors(&["new"], 0.0).unwrap();
        assert!(
            !neighbors.is_empty(),
            "New node should have inherited edges"
        );
        let neighbor_ids: Vec<&str> = neighbors.iter().map(|(id, _, _)| id.as_str()).collect();
        assert!(neighbor_ids.contains(&"friend"));
    }

    #[test]
    fn test_remove_node_preserves_incoming_supersedes() {
        // Bug #4: remove_node should preserve incoming SUPERSEDES edges for lineage
        let graph = open_test_graph();
        let now = chrono::Utc::now().timestamp();
        graph.add_node("new", Some("proj"), now).unwrap();
        graph.add_node("old", Some("proj"), now).unwrap();

        // the replace workflow: SUPERSEDES edge new -> old, then the old node goes
        graph.add_supersedes_edge("new", "old").unwrap();
        graph.remove_node("old").unwrap();

        let edges: Vec<(String, String, String)> = graph
            .conn
            .prepare("SELECT from_id, to_id, edge_type FROM graph_edges")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert_eq!(
            edges,
            [("new".into(), "old".into(), "supersedes".into())],
            "SUPERSEDES edge should be preserved"
        );
    }

    #[test]
    fn test_get_neighbors_mapped() {
        let graph = open_test_graph();
        let now = chrono::Utc::now().timestamp();
        graph.add_node("A", Some("proj"), now).unwrap();
        graph.add_node("B", Some("proj"), now).unwrap();
        graph.add_node("C", Some("proj"), now).unwrap();
        graph.add_node("D", Some("proj"), now).unwrap();

        graph.add_related_edge("A", "C", 0.9, "test").unwrap();
        graph.add_related_edge("B", "D", 0.9, "test").unwrap();

        let map = graph.get_neighbors_mapped(&["A", "B"], 0.0).unwrap();

        // A should have C as neighbor, B should have D
        assert!(map.get("A").unwrap().contains(&"C".to_string()));
        assert!(map.get("B").unwrap().contains(&"D".to_string()));
        // Input IDs should not appear as neighbors
        for neighbors in map.values() {
            assert!(!neighbors.contains(&"A".to_string()));
            assert!(!neighbors.contains(&"B".to_string()));
        }
    }
}
