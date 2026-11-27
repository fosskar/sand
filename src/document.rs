//! Content type for chunking
//!
//! This module is kept for the ContentType enum used by the chunker.

/// Content type of content (used for smart chunking)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContentType {
    /// Plain text content
    #[default]
    Text,
    /// Markdown formatted content
    Markdown,
    /// JSON data
    Json,
    /// YAML data
    Yaml,
    /// Source code
    Code,
}
