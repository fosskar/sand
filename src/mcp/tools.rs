//! MCP Tool definitions for sand
//!
//! 4 tools: store, recall, list, forget

use serde::Deserialize;
use serde_json::{Value, json};

use crate::backend::{Backend, BackendError, StoreRequest, truncate};
use crate::retry::{RetryConfig, with_retry};
use crate::{ListScope, StoreScope};

use super::protocol::{CallToolResult, Tool};
use super::server::ServerContext;

/// Get all available tools (4 total)
pub fn get_tools() -> Vec<Tool> {
    let store_schema = {
        #[allow(unused_mut)]
        let mut props = json!({
            "content": {
                "type": "string",
                "description": "The content to store"
            },
            "scope": {
                "type": "string",
                "enum": ["project", "global"],
                "default": "project",
                "description": "Where to store: 'project' (facts specific to the current repository) or 'global' (everything else: user preferences and general knowledge that applies across projects)"
            },
            "replace_id": {
                "type": "string",
                "description": "ID of an existing item to replace. Atomically stores new content and deletes the old item, preserving graph lineage."
            }
        });

        #[cfg(feature = "bench")]
        {
            props.as_object_mut().unwrap().insert(
                "created_at".to_string(),
                json!({
                    "type": "number",
                    "description": "Override creation timestamp (Unix seconds). Benchmark builds only."
                }),
            );
        }

        json!({
            "type": "object",
            "properties": props,
            "required": ["content"]
        })
    };

    vec![
        Tool {
            name: "store".to_string(),
            description: "Store content in the user's long-term memory, shared by all agents and sessions. Call this without being asked when the user states a preference or decision, says to remember something, or you learn a durable non-obvious fact. Use scope 'global' for anything not specific to the current repository: facts about the user, their preferences, and general knowledge such as how a tool or language behaves. Use the default 'project' scope for facts about the current repository. Long content is automatically chunked for better search. Use replace_id to atomically replace an existing item.".to_string(),
            input_schema: store_schema,
        },
        Tool {
            name: "recall".to_string(),
            description: "Search the user's long-term memory by semantic similarity, across project and global items. Call this at the start of a conversation and before work where earlier context could matter: user preferences, prior decisions, project conventions. Returns matching items with relevant excerpts for chunked content.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "What to search for (semantic search)"
                    },
                    "limit": {
                        "type": "number",
                        "default": 5,
                        "description": "Maximum number of results"
                    }
                },
                "required": ["query"]
            }),
        },
        Tool {
            name: "list".to_string(),
            description: "List stored items.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "number",
                        "default": 10,
                        "description": "Maximum number of results"
                    },
                    "scope": {
                        "type": "string",
                        "enum": ["project", "global", "all"],
                        "default": "project",
                        "description": "Which items to list: 'project', 'global', or 'all'"
                    }
                }
            }),
        },
        Tool {
            name: "forget".to_string(),
            description: "Delete a stored item by its ID.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The item ID to delete"
                    }
                },
                "required": ["id"]
            }),
        },
    ]
}

// ========== Parameter Structs ==========

#[derive(Debug, Deserialize)]
pub struct StoreParams {
    pub content: String,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub replace_id: Option<String>,
    /// Override creation timestamp (Unix seconds). Benchmark builds only.
    #[cfg(feature = "bench")]
    #[serde(default)]
    pub created_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct RecallParams {
    pub query: String,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct ListParams {
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ForgetParams {
    pub id: String,
}

// ========== Tool Execution ==========

pub async fn execute_tool<B: Backend>(
    ctx: &ServerContext<B>,
    name: &str,
    args: Option<Value>,
) -> CallToolResult {
    let config = RetryConfig::default();
    let args_for_retry = args.clone();

    let result = with_retry(&config, || {
        let backend = &ctx.backend;
        let name_ref = name;
        let args_clone = args_for_retry.clone();

        async move {
            let result = match name_ref {
                "store" => execute_store(backend, args_clone).await,
                "recall" => execute_recall(backend, args_clone).await,
                "list" => execute_list(backend, args_clone).await,
                "forget" => execute_forget(backend, args_clone).await,
                _ => return Ok(CallToolResult::error(format!("Unknown tool: {}", name_ref))),
            };

            if result.is_error.unwrap_or(false)
                && let Some(content) = result.content.first()
                && is_retryable_error(&content.text)
            {
                return Err(content.text.clone());
            }

            Ok(result)
        }
    })
    .await;

    match result {
        Ok(call_result) => call_result,
        Err(e) => {
            tracing::error!("Operation failed after retries: {}", e);
            CallToolResult::error("Operation failed after retries")
        }
    }
}

fn is_retryable_error(error_msg: &str) -> bool {
    let retryable_patterns = [
        "connection",
        "timeout",
        "temporarily unavailable",
        "resource busy",
        "lock",
        "I/O error",
        "Failed to open",
        "Failed to connect",
    ];

    let lower = error_msg.to_lowercase();
    retryable_patterns
        .iter()
        .any(|p| lower.contains(&p.to_lowercase()))
}

// ========== Tool Implementations ==========

async fn execute_store(backend: &impl Backend, args: Option<Value>) -> CallToolResult {
    let params: StoreParams = match args {
        Some(v) => match serde_json::from_value(v) {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!("Parameter validation failed: {}", e);
                return CallToolResult::error("Invalid parameters");
            }
        },
        None => return CallToolResult::error("Missing parameters"),
    };

    if params.content.trim().is_empty() {
        return CallToolResult::error("Content must not be empty");
    }

    // Reject oversized content to prevent OOM during embedding/chunking.
    // Intentionally byte-based (not char-based): memory allocation is proportional
    // to byte length, so this is the correct metric for OOM prevention.
    const MAX_CONTENT_BYTES: usize = 1_000_000;
    if params.content.len() > MAX_CONTENT_BYTES {
        return CallToolResult::error(format!(
            "Content too large: {} bytes (max {} bytes)",
            params.content.len(),
            MAX_CONTENT_BYTES
        ));
    }

    // Parse scope
    let scope = params
        .scope
        .as_deref()
        .map(|s| s.parse::<StoreScope>())
        .transpose();

    let scope = match scope {
        Ok(s) => s.unwrap_or(StoreScope::Project),
        Err(e) => return CallToolResult::error(e),
    };

    #[cfg(feature = "bench")]
    let created_at = params.created_at;
    #[cfg(not(feature = "bench"))]
    let created_at = None;

    let request = StoreRequest {
        content: params.content,
        scope,
        replace_id: params.replace_id,
        created_at,
    };

    match backend.store(request).await {
        Ok(stored) => {
            let message = match stored.replaced_id {
                Some(ref old_id) => format!("Stored in {} scope (replaced {})", scope, old_id),
                None => format!("Stored in {} scope", scope),
            };

            let mut result = json!({
                "success": true,
                "id": stored.id,
                "message": message
            });

            if stored.replaced_id.is_some() {
                result["replaced_id"] = json!(stored.replaced_id);
            }

            if !stored.potential_conflicts.is_empty() {
                let conflicts: Vec<Value> = stored
                    .potential_conflicts
                    .iter()
                    .map(|c| {
                        json!({
                            "id": c.id,
                            "content": c.content,
                            "similarity": format!("{:.2}", c.similarity)
                        })
                    })
                    .collect();
                result["potential_conflicts"] = json!(conflicts);
            }

            CallToolResult::success(
                serde_json::to_string_pretty(&result)
                    .unwrap_or_else(|e| format!("{{\"error\": \"serialization failed: {}\"}}", e)),
            )
        }
        Err(e) => backend_error(e),
    }
}

async fn execute_recall(backend: &impl Backend, args: Option<Value>) -> CallToolResult {
    let params: RecallParams = match args {
        Some(v) => match serde_json::from_value(v) {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!("Parameter validation failed: {}", e);
                return CallToolResult::error("Invalid parameters");
            }
        },
        None => return CallToolResult::error("Missing parameters"),
    };

    if params.query.trim().is_empty() {
        return CallToolResult::error("Query must not be empty");
    }

    // Reject oversized queries to prevent OOM during tokenization.
    // The model truncates to 512 tokens (~2KB of English text), so anything
    // beyond 10KB is wasted processing.
    const MAX_QUERY_BYTES: usize = 10_000;
    if params.query.len() > MAX_QUERY_BYTES {
        return CallToolResult::error(format!(
            "Query too large: {} bytes (max {} bytes)",
            params.query.len(),
            MAX_QUERY_BYTES
        ));
    }

    let limit = params.limit.unwrap_or(5).min(100);

    let recalled = match backend.recall(&params.query, limit).await {
        Ok(r) => r,
        Err(e) => return backend_error(e),
    };
    let recall_result = &recalled.result;
    let neighbors_map = &recalled.related_ids;

    if recall_result.results.is_empty() {
        return CallToolResult::success("No items found matching your query.");
    }

    let results = &recall_result.results;

    let formatted: Vec<Value> = results
        .iter()
        .map(|r| {
            let mut obj = json!({
                "id": r.id,
                "content": r.content,
                "similarity": format!("{:.2}", r.similarity),
                "created": r.created_at.to_rfc3339(),
            });

            // Include raw (pre-decay) similarity when decay scoring was applied
            if let Some(&raw_sim) = recall_result.raw_similarities.get(&r.id)
                && (raw_sim - r.similarity).abs() > 0.001
            {
                obj["raw_similarity"] = json!(format!("{:.2}", raw_sim));
            }

            if let Some(ref excerpt) = r.relevant_excerpt {
                obj["relevant_excerpt"] = json!(excerpt);
            }

            // Cross-project flag
            if let Some(current_pid) = backend.project_id()
                && let Some(ref item_pid) = r.project_id
                && item_pid != current_pid
            {
                obj["cross_project"] = json!(true);
            }

            // Related IDs from graph (batch lookup)
            if let Some(related) = neighbors_map.get(&r.id)
                && !related.is_empty()
            {
                obj["related_ids"] = json!(related);
            }

            obj
        })
        .collect();

    let mut result_json = json!({
        "count": results.len(),
        "results": formatted
    });

    if !recall_result.graph_expanded.is_empty() {
        result_json["graph_expanded"] = json!(recall_result.graph_expanded);
    }

    if !recall_result.suggested.is_empty() {
        result_json["suggested"] = json!(recall_result.suggested);
    }

    CallToolResult::success(
        serde_json::to_string_pretty(&result_json)
            .unwrap_or_else(|e| format!("{{\"error\": \"serialization failed: {}\"}}", e)),
    )
}

async fn execute_list(backend: &impl Backend, args: Option<Value>) -> CallToolResult {
    let params: ListParams =
        args.and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or(ListParams {
                limit: None,
                scope: None,
            });

    let limit = params.limit.unwrap_or(10).min(100);

    let scope = params
        .scope
        .as_deref()
        .map(|s| s.parse::<ListScope>())
        .transpose();

    let scope = match scope {
        Ok(s) => s.unwrap_or(ListScope::Project),
        Err(e) => return CallToolResult::error(e),
    };

    match backend.list(limit, scope).await {
        Ok(items) => {
            if items.is_empty() {
                CallToolResult::success("No items stored yet.")
            } else {
                let formatted: Vec<Value> = items
                    .iter()
                    .map(|item| {
                        let content_preview = truncate(&item.content, 100);
                        let mut obj = json!({
                            "id": item.id,
                            "content": content_preview,
                            "created": item.created_at.to_rfc3339(),
                        });

                        if item.is_chunked {
                            obj["chunked"] = json!(true);
                        }

                        obj
                    })
                    .collect();

                let result = json!({
                    "count": items.len(),
                    "items": formatted
                });

                CallToolResult::success(
                    serde_json::to_string_pretty(&result).unwrap_or_else(|e| {
                        format!("{{\"error\": \"serialization failed: {}\"}}", e)
                    }),
                )
            }
        }
        Err(e) => backend_error(e),
    }
}

async fn execute_forget(backend: &impl Backend, args: Option<Value>) -> CallToolResult {
    let params: ForgetParams = match args {
        Some(v) => match serde_json::from_value(v) {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!("Parameter validation failed: {}", e);
                return CallToolResult::error("Invalid parameters");
            }
        },
        None => return CallToolResult::error("Missing parameters"),
    };

    match backend.forget(&params.id).await {
        Ok(()) => {
            let result = json!({
                "success": true,
                "message": format!("Deleted item: {}", params.id)
            });
            CallToolResult::success(
                serde_json::to_string_pretty(&result)
                    .unwrap_or_else(|e| format!("{{\"error\": \"serialization failed: {}\"}}", e)),
            )
        }
        Err(e) => backend_error(e),
    }
}

// ========== Utilities ==========

/// Turn a backend error into a tool error. Failures are logged in detail and reported
/// to the MCP client by their context only, so no file paths or database internals leak.
fn backend_error(err: BackendError) -> CallToolResult {
    match err {
        BackendError::Failed { context, error } => {
            tracing::error!("{}: {}", context, error);
            CallToolResult::error(context)
        }
        err => CallToolResult::error(err.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use crate::backend::LocalBackend;

    /// Create a ServerContext with temp dirs for integration testing.
    async fn setup_test_context() -> (ServerContext<LocalBackend>, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = test_context(tmp.path(), "test-project-00000001");
        (ctx, tmp)
    }

    /// A ServerContext on the store in `dir`, acting for `project_id`.
    fn test_context(dir: &Path, project_id: &str) -> ServerContext<LocalBackend> {
        let embedder = Arc::new(crate::Embedder::new().unwrap());
        ServerContext {
            backend: LocalBackend::new(dir.join("data"), Some(project_id.to_string()))
                .with_background_work(embedder),
            rate_limit: Mutex::new(super::super::server::RateLimitState {
                window_start_ms: 0,
                count: 0,
            }),
        }
    }

    #[tokio::test]
    #[ignore] // requires model download
    async fn test_store_and_recall_roundtrip() {
        let (ctx, _tmp) = setup_test_context().await;

        // Store an item
        let store_result = execute_tool(
            &ctx,
            "store",
            Some(json!({ "content": "Rust is a systems programming language" })),
        )
        .await;
        assert!(
            store_result.is_error.is_none(),
            "Store should succeed: {:?}",
            store_result.content
        );

        // Recall by query
        let recall_result = execute_tool(
            &ctx,
            "recall",
            Some(json!({ "query": "systems programming language" })),
        )
        .await;
        assert!(recall_result.is_error.is_none(), "Recall should succeed");

        let text = &recall_result.content[0].text;
        assert!(
            text.contains("Rust is a systems programming language"),
            "Recall should return stored content, got: {}",
            text
        );
    }

    #[tokio::test]
    #[ignore] // requires model download
    async fn test_store_and_list() {
        let (ctx, _tmp) = setup_test_context().await;

        // Store 2 items
        execute_tool(
            &ctx,
            "store",
            Some(json!({ "content": "First item for listing" })),
        )
        .await;
        execute_tool(
            &ctx,
            "store",
            Some(json!({ "content": "Second item for listing" })),
        )
        .await;

        // List items
        let list_result = execute_tool(&ctx, "list", Some(json!({ "scope": "project" }))).await;
        assert!(list_result.is_error.is_none(), "List should succeed");

        let text = &list_result.content[0].text;
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["count"], 2, "Should list 2 items");
    }

    #[tokio::test]
    #[ignore] // requires model download
    async fn test_store_conflict_detection() {
        let (ctx, _tmp) = setup_test_context().await;

        // Store first item
        execute_tool(
            &ctx,
            "store",
            Some(json!({ "content": "The quick brown fox jumps over the lazy dog" })),
        )
        .await;

        // Store nearly identical item
        let result = execute_tool(
            &ctx,
            "store",
            Some(json!({ "content": "The quick brown fox jumps over the lazy dog" })),
        )
        .await;
        assert!(result.is_error.is_none(), "Store should succeed");

        let text = &result.content[0].text;
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert!(
            parsed.get("potential_conflicts").is_some(),
            "Should detect conflict for near-duplicate content, got: {}",
            text
        );
    }

    #[tokio::test]
    #[ignore] // requires model download
    async fn test_forget_removes_item() {
        let (ctx, _tmp) = setup_test_context().await;

        // Store an item
        let store_result = execute_tool(
            &ctx,
            "store",
            Some(json!({ "content": "Item to be forgotten" })),
        )
        .await;
        let text = &store_result.content[0].text;
        let parsed: Value = serde_json::from_str(text).unwrap();
        let item_id = parsed["id"].as_str().unwrap().to_string();

        // Forget it
        let forget_result = execute_tool(&ctx, "forget", Some(json!({ "id": item_id }))).await;
        assert!(forget_result.is_error.is_none(), "Forget should succeed");

        // List should be empty
        let list_result = execute_tool(&ctx, "list", Some(json!({ "scope": "project" }))).await;
        let text = &list_result.content[0].text;
        assert!(
            text.contains("No items stored yet"),
            "Should have no items after forget, got: {}",
            text
        );
    }

    #[tokio::test]
    #[ignore] // requires model download
    async fn test_recall_empty_db() {
        let (ctx, _tmp) = setup_test_context().await;

        let result = execute_tool(&ctx, "recall", Some(json!({ "query": "anything" }))).await;
        assert!(
            result.is_error.is_none(),
            "Recall on empty DB should not error"
        );

        let text = &result.content[0].text;
        assert!(
            text.contains("No items found"),
            "Should indicate no items found, got: {}",
            text
        );
    }

    #[tokio::test]
    #[ignore] // requires model download
    async fn test_store_rejects_oversized_content() {
        let (ctx, _tmp) = setup_test_context().await;

        let large_content = "x".repeat(1_100_000); // >1MB
        let result = execute_tool(&ctx, "store", Some(json!({ "content": large_content }))).await;
        assert!(
            result.is_error == Some(true),
            "Should reject oversized content"
        );

        let text = &result.content[0].text;
        assert!(
            text.contains("too large"),
            "Error should mention size, got: {}",
            text
        );
    }

    #[tokio::test]
    #[ignore] // requires model download
    async fn test_recall_rejects_oversized_query() {
        let (ctx, _tmp) = setup_test_context().await;

        let large_query = "x".repeat(11_000); // >10KB
        let result = execute_tool(&ctx, "recall", Some(json!({ "query": large_query }))).await;
        assert!(
            result.is_error == Some(true),
            "Should reject oversized query"
        );

        let text = &result.content[0].text;
        assert!(
            text.contains("too large"),
            "Error should mention size, got: {}",
            text
        );
    }

    #[tokio::test]
    #[ignore] // requires model download
    async fn test_store_missing_params() {
        let (ctx, _tmp) = setup_test_context().await;

        // No params at all
        let result = execute_tool(&ctx, "store", None).await;
        assert!(result.is_error == Some(true), "Should error with no params");

        // Empty object (missing required 'content')
        let result = execute_tool(&ctx, "store", Some(json!({}))).await;
        assert!(
            result.is_error == Some(true),
            "Should error with missing content"
        );
    }

    #[tokio::test]
    #[ignore] // requires model download
    async fn test_store_replace_id() {
        let (ctx, _tmp) = setup_test_context().await;

        // Store an item
        let store_result = execute_tool(
            &ctx,
            "store",
            Some(json!({ "content": "Original content to replace" })),
        )
        .await;
        assert!(store_result.is_error.is_none(), "Store should succeed");
        let text = &store_result.content[0].text;
        let parsed: Value = serde_json::from_str(text).unwrap();
        let old_id = parsed["id"].as_str().unwrap().to_string();

        // Store with replace_id
        let replace_result = execute_tool(
            &ctx,
            "store",
            Some(json!({
                "content": "Updated replacement content",
                "replace_id": old_id
            })),
        )
        .await;
        assert!(replace_result.is_error.is_none(), "Replace should succeed");
        let text = &replace_result.content[0].text;
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert!(
            parsed["replaced_id"].is_string(),
            "Should include replaced_id"
        );
        assert_eq!(parsed["replaced_id"].as_str().unwrap(), old_id);
        assert!(
            parsed["message"].as_str().unwrap().contains("replaced"),
            "Message should mention replacement"
        );

        // Old item should be gone
        let list_result = execute_tool(&ctx, "list", Some(json!({ "scope": "project" }))).await;
        let text = &list_result.content[0].text;
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(
            parsed["count"], 1,
            "Should have exactly 1 item after replace"
        );
    }

    fn store_id(result: &CallToolResult) -> String {
        assert!(
            result.is_error.is_none(),
            "store failed: {:?}",
            result.content
        );
        let parsed: Value = serde_json::from_str(&result.content[0].text).unwrap();
        parsed["id"].as_str().unwrap().to_string()
    }

    async fn list_ids(ctx: &ServerContext<LocalBackend>, scope: &str) -> Vec<String> {
        let result = execute_tool(ctx, "list", Some(json!({ "scope": scope }))).await;
        assert!(
            result.is_error.is_none(),
            "list failed: {:?}",
            result.content
        );
        let text = &result.content[0].text;
        if text == "No items stored yet." {
            return Vec::new();
        }
        let parsed: Value = serde_json::from_str(text).unwrap();
        parsed["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["id"].as_str().unwrap().to_string())
            .collect()
    }

    #[tokio::test]
    #[ignore] // requires model download
    async fn test_store_global_scope_is_global() {
        let tmp = tempfile::TempDir::new().unwrap();
        // list rejects project ids that are not hex
        let ctx = test_context(tmp.path(), "00000000000000000000000000000000000000aa");

        let result = execute_tool(
            &ctx,
            "store",
            Some(json!({ "content": "the user prefers tabs over spaces", "scope": "global" })),
        )
        .await;
        let id = store_id(&result);

        assert_eq!(list_ids(&ctx, "global").await, vec![id.clone()]);
        assert!(list_ids(&ctx, "project").await.is_empty());
        assert_eq!(list_ids(&ctx, "all").await, vec![id.clone()]);

        // another project sees the item as global: no cross-project flag, may forget it
        let ctx = test_context(tmp.path(), "00000000000000000000000000000000000000bb");
        let recall = execute_tool(
            &ctx,
            "recall",
            Some(json!({ "query": "the user prefers tabs over spaces" })),
        )
        .await;
        assert!(
            recall.is_error.is_none(),
            "recall failed: {:?}",
            recall.content
        );
        let parsed: Value = serde_json::from_str(&recall.content[0].text).unwrap();
        let hit = parsed["results"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == id.as_str())
            .expect("recall should return the global item");
        assert!(hit.get("cross_project").is_none(), "got: {}", hit);

        let forget = execute_tool(&ctx, "forget", Some(json!({ "id": id }))).await;
        assert!(
            forget.is_error.is_none(),
            "forget failed: {:?}",
            forget.content
        );
        assert!(list_ids(&ctx, "all").await.is_empty());
    }

    #[tokio::test]
    #[ignore] // requires model download
    async fn test_store_replace_id_global_scope() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = test_context(tmp.path(), "00000000000000000000000000000000000000aa");

        let old_id = store_id(
            &execute_tool(
                &ctx,
                "store",
                Some(json!({ "content": "global note, first version", "scope": "global" })),
            )
            .await,
        );
        let new_id = store_id(
            &execute_tool(
                &ctx,
                "store",
                Some(json!({
                    "content": "global note, second version",
                    "scope": "global",
                    "replace_id": old_id
                })),
            )
            .await,
        );

        assert_eq!(list_ids(&ctx, "global").await, vec![new_id]);
        assert!(list_ids(&ctx, "project").await.is_empty());
    }

    #[tokio::test]
    #[ignore] // requires model download
    async fn test_store_replace_id_invalid() {
        let (ctx, _tmp) = setup_test_context().await;

        // Store with an invalid replace_id — should still succeed (non-fatal)
        let result = execute_tool(
            &ctx,
            "store",
            Some(json!({
                "content": "Content with bad replace_id",
                "replace_id": "not a valid id!@#$"
            })),
        )
        .await;
        assert!(
            result.is_error.is_none(),
            "Store should succeed even with invalid replace_id"
        );
        let text = &result.content[0].text;
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert!(
            parsed["success"].as_bool().unwrap(),
            "Should report success"
        );
        // Should NOT have replaced_id since the ID was invalid
        assert!(
            parsed.get("replaced_id").is_none(),
            "Should not include replaced_id for invalid ID"
        );
    }
}
