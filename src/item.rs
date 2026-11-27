//! Unified Item type for semantic storage
//!
//! Items unify memories and documents into a single concept with automatic chunking.

use chrono::{DateTime, Utc};

/// A unified item stored in sand
#[derive(Debug, Clone)]
pub struct Item {
    /// Unique identifier (UUID)
    pub id: String,
    /// The actual content
    pub content: String,
    /// Vector embedding
    pub embedding: Vec<f32>,
    /// Project ID (None for global items)
    pub project_id: Option<String>,
    /// Whether this item was chunked (internal)
    pub is_chunked: bool,
    /// When this item was created
    pub created_at: DateTime<Utc>,
}

impl Item {
    /// Create a new item with content
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            content: content.into(),
            embedding: Vec::new(),
            project_id: None,
            is_chunked: false,
            created_at: Utc::now(),
        }
    }

    /// Set the project ID
    pub fn with_project_id(mut self, project_id: impl Into<String>) -> Self {
        self.project_id = Some(project_id.into());
        self
    }

    /// Override the creation timestamp (benchmark builds only)
    #[cfg(feature = "bench")]
    pub fn with_created_at(mut self, created_at: DateTime<Utc>) -> Self {
        self.created_at = created_at;
        self
    }

    /// Get the text to embed for this item
    /// For chunked items: first ~500 chars
    /// For non-chunked items: full content
    pub fn embedding_text(&self) -> String {
        if self.is_chunked {
            self.content.chars().take(500).collect()
        } else {
            self.content.clone()
        }
    }
}

/// A chunk of an item (internal, not exposed to MCP)
#[derive(Debug, Clone)]
pub struct Chunk {
    /// Unique identifier (UUID)
    pub id: String,
    /// Parent item ID
    pub item_id: String,
    /// Index of this chunk within the item (0-based)
    pub chunk_index: usize,
    /// The chunk content
    pub content: String,
    /// Vector embedding of the chunk
    pub embedding: Vec<f32>,
    /// Optional context (e.g., header path, function name)
    pub context: Option<String>,
}

impl Chunk {
    /// Create a new chunk
    pub fn new(item_id: impl Into<String>, chunk_index: usize, content: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            item_id: item_id.into(),
            chunk_index,
            content: content.into(),
            embedding: Vec::new(),
            context: None,
        }
    }

    /// Set context
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(context.into());
        self
    }
}

/// Result from storing an item
#[derive(Debug, Clone)]
pub struct StoreResult {
    /// The ID of the newly stored item
    pub id: String,
    /// Potentially conflicting items (high similarity)
    pub potential_conflicts: Vec<ConflictInfo>,
}

/// Information about a potential conflict
#[derive(Debug, Clone)]
pub struct ConflictInfo {
    /// The ID of the conflicting item
    pub id: String,
    /// The content of the conflicting item
    pub content: String,
    /// Similarity score (0.0-1.0)
    pub similarity: f32,
}

/// Result from a search query
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// The matching item's id
    pub id: String,
    /// Content (full if short, or preview if chunked)
    pub content: String,
    /// Most relevant chunk content (if chunked)
    pub relevant_excerpt: Option<String>,
    /// Similarity score (0.0-1.0, higher is more similar)
    pub similarity: f32,
    /// When created
    pub created_at: DateTime<Utc>,
    /// Project ID, for cross-project checks and graph backfill
    pub project_id: Option<String>,
}

impl SearchResult {
    /// Create from an item (non-chunked)
    pub fn from_item(item: &Item, similarity: f32) -> Self {
        Self {
            id: item.id.clone(),
            content: item.content.clone(),
            relevant_excerpt: None,
            similarity,
            created_at: item.created_at,
            project_id: item.project_id.clone(),
        }
    }

    /// Create from an item with chunk excerpt
    pub fn from_item_with_excerpt(item: &Item, similarity: f32, excerpt: String) -> Self {
        // For chunked items, show a preview of the content
        let content: String = item.content.chars().take(100).collect();
        Self {
            id: item.id.clone(),
            content,
            relevant_excerpt: Some(excerpt),
            similarity,
            created_at: item.created_at,
            project_id: item.project_id.clone(),
        }
    }
}

/// Filters for search/list queries
#[derive(Debug, Default, Clone)]
pub struct ItemFilters {
    /// Minimum similarity threshold (0.0-1.0)
    pub min_similarity: Option<f32>,
}

impl ItemFilters {
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(test)]
impl Item {
    /// Mark as chunked (test only)
    pub fn with_chunked(mut self, is_chunked: bool) -> Self {
        self.is_chunked = is_chunked;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedding_text_chunked() {
        let item = Item::new("a".repeat(1000)).with_chunked(true);
        let text = item.embedding_text();
        assert_eq!(text.len(), 500);
    }
}
