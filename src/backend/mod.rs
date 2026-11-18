//! The operations the CLI and the MCP server run against a memory store.

mod local;

pub(crate) use local::truncate;
pub use local::{RecallConfig, RecallResult, recall_pipeline};
