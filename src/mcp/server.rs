//! MCP Server implementation

use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::io::{self, BufRead, Read, Write};
use std::sync::Mutex;
use tokio::runtime::Runtime;

use crate::backend::Backend;

use super::protocol::{
    CallToolParams, INVALID_PARAMS, INVALID_REQUEST, InitializeResult, ListToolsResult,
    MCP_VERSION, METHOD_NOT_FOUND, PARSE_ERROR, Request, Response, ServerCapabilities, ServerInfo,
    ToolsCapability,
};
use super::tools::{execute_tool, get_tools};

/// Rate limiter state, protected by a mutex to avoid race conditions
/// between window reset and count increment.
pub struct RateLimitState {
    pub window_start_ms: u64,
    pub count: u64,
}

/// Server context holding shared resources
pub struct ServerContext<B> {
    /// Store the tools act on
    pub backend: B,
    /// Rate limiter state (mutex protects window+count as a unit)
    pub rate_limit: Mutex<RateLimitState>,
}

/// Run the MCP server on stdio
pub fn run<B: Backend>(backend: B) -> Result<()> {
    // Create tokio runtime for async operations
    let rt = Runtime::new().context("Failed to create tokio runtime")?;

    // Create server context with shared resources
    let ctx = ServerContext {
        backend,
        rate_limit: Mutex::new(RateLimitState {
            window_start_ms: 0,
            count: 0,
        }),
    };

    // Run the request loop in a block so borrows of `rt` end before shutdown
    {
        let stdin = io::stdin();
        let mut stdout = io::stdout();
        let mut reader = stdin.lock();

        // Maximum line size to prevent OOM from a malicious client sending
        // a single huge line without a newline character.
        const MAX_LINE_BYTES: usize = 10 * 1024 * 1024; // 10 MB

        tracing::info!("MCP server ready, waiting for requests...");

        let mut line = String::new();
        loop {
            line.clear();
            // Use Read::take to bound memory before the newline is found.
            // Without this, read_line would buffer an unlimited amount of data
            // if the client never sends a newline character.
            let bytes_read = (&mut reader)
                .take((MAX_LINE_BYTES + 1) as u64)
                .read_line(&mut line)
                .context("Failed to read line")?;
            if bytes_read == 0 {
                break; // EOF
            }

            if line.len() > MAX_LINE_BYTES
                || (line.len() >= MAX_LINE_BYTES && !line.ends_with('\n'))
            {
                tracing::error!(
                    "Rejecting oversized request: {} bytes (max {})",
                    line.len(),
                    MAX_LINE_BYTES
                );
                // Drain remaining bytes until newline or EOF to resync the stream.
                // Use a fixed-size buffer to avoid unbounded allocation from a
                // malicious client that never sends a newline.
                if !line.ends_with('\n') {
                    let mut drain_buf = [0u8; 8192];
                    let mut drained = 0usize;
                    const MAX_DRAIN_BYTES: usize = 100 * 1024 * 1024; // 100 MB
                    loop {
                        match reader.read(&mut drain_buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                drained += n;
                                if drain_buf[..n].contains(&b'\n') || drained >= MAX_DRAIN_BYTES {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                }
                let response = Response::error(None, INVALID_REQUEST, "Request too large");
                let response_json = serde_json::to_string(&response)?;
                writeln!(stdout, "{}", response_json)?;
                stdout.flush()?;
                continue;
            }

            if line.trim().is_empty() {
                continue;
            }

            tracing::debug!(
                "Received: ({} bytes) {}",
                line.len(),
                if line.len() > 200 {
                    &line[..200]
                } else {
                    &line
                }
            );

            // Handle request and get optional response (notifications don't get responses)
            if let Some(response) = handle_request(&rt, &ctx, &line) {
                let response_json = serde_json::to_string(&response)?;
                tracing::debug!("Sending: {}", response_json);

                writeln!(stdout, "{}", response_json)?;
                stdout.flush()?;
            }
        }
    }

    // Graceful shutdown: wait for in-flight background tasks before dropping
    tracing::info!("Client disconnected, shutting down...");
    let _ = rt.block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(10), ctx.backend.finish()).await
    });
    drop(ctx);
    rt.shutdown_timeout(std::time::Duration::from_secs(5));

    Ok(())
}

/// Handle a single request (returns None for notifications)
fn handle_request<B: Backend>(
    rt: &Runtime,
    ctx: &ServerContext<B>,
    line: &str,
) -> Option<Response> {
    // Parse request
    let request: Request = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Parse error: {}", e);
            return Some(Response::error(
                None,
                PARSE_ERROR,
                "Parse error: invalid JSON-RPC request",
            ));
        }
    };

    // Validate JSON-RPC version
    if request.jsonrpc != "2.0" {
        return Some(Response::error(
            request.id,
            INVALID_REQUEST,
            "Invalid JSON-RPC version, expected 2.0",
        ));
    }

    // Check if this is a notification (no id = notification, don't respond)
    let is_notification = request.id.is_none();

    // Route to handler
    match request.method.as_str() {
        "initialize" => Some(handle_initialize(request.id)),
        "notifications/initialized" | "initialized" => {
            tracing::info!("Client initialized");
            None // Notifications don't get responses
        }
        "tools/list" => Some(handle_list_tools(request.id)),
        "tools/call" => Some(handle_call_tool(rt, ctx, request.id, request.params)),
        "ping" => Some(handle_ping(request.id)),
        _ => {
            // For unknown notifications, just ignore them
            if is_notification {
                tracing::debug!("Ignoring unknown notification: {}", request.method);
                None
            } else {
                tracing::warn!("Unknown method: {}", request.method);
                Some(Response::error(
                    request.id,
                    METHOD_NOT_FOUND,
                    &format!("Unknown method: {}", request.method),
                ))
            }
        }
    }
}

/// Handle initialize request
fn handle_initialize(id: Option<Value>) -> Response {
    let instructions = "\
sand is the user's long-term memory, shared by all agents and sessions: recall before work where earlier context could matter, and store preferences, decisions and durable facts (scope global unless specific to the current repository). All data stays on disk — no API keys required.

Environment variables (set before starting the server or CLI):
- SAND_DB: Override database path (default: ~/.sand/data). \
Useful for ephemeral or isolated storage, e.g. SAND_DB=/tmp/task-xyz sand store \"...\".
- SAND_EMBEDDING_MODEL: Override embedding model (default: all-MiniLM-L6-v2). \
Options: all-MiniLM-L6-v2, bge-small-en-v1.5.

sand is also available as a CLI: sand store, sand recall, sand list, sand forget. \
Run `sand --help` for details."
        .to_string();

    let result = InitializeResult {
        protocol_version: MCP_VERSION.to_string(),
        capabilities: ServerCapabilities {
            tools: ToolsCapability {
                list_changed: false,
            },
        },
        server_info: ServerInfo {
            name: "sand".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
        instructions: Some(instructions),
    };

    // InitializeResult is a simple struct; serialization is infallible.
    Response::success(
        id,
        serde_json::to_value(result).expect("InitializeResult serialization"),
    )
}

/// Handle tools/list request
fn handle_list_tools(id: Option<Value>) -> Response {
    let result = ListToolsResult { tools: get_tools() };

    // ListToolsResult is a simple struct; serialization is infallible.
    Response::success(
        id,
        serde_json::to_value(result).expect("ListToolsResult serialization"),
    )
}

/// Handle tools/call request
fn handle_call_tool<B: Backend>(
    rt: &Runtime,
    ctx: &ServerContext<B>,
    id: Option<Value>,
    params: Option<Value>,
) -> Response {
    let params: CallToolParams = match params {
        Some(p) => match serde_json::from_value(p) {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!("Invalid tool call params: {}", e);
                return Response::error(id, INVALID_PARAMS, "Invalid params");
            }
        },
        None => {
            return Response::error(id, INVALID_PARAMS, "Missing params");
        }
    };

    tracing::info!("Calling tool: {}", params.name);

    // Safety rate limit: guard against runaway client loops hammering IO.
    // Generous limit since this is a local-only single-client server.
    {
        const MAX_CALLS_PER_MINUTE: u64 = 600;
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let mut state = ctx.rate_limit.lock().unwrap_or_else(|e| {
            tracing::warn!("Rate limiter mutex was poisoned, recovering with reset");
            let mut guard = e.into_inner();
            // Reset to a clean state since the previous holder panicked mid-update
            guard.window_start_ms = 0;
            guard.count = 0;
            guard
        });
        if now_ms.saturating_sub(state.window_start_ms) > 60_000 {
            // Window expired, reset
            state.window_start_ms = now_ms;
            state.count = 0;
        }
        state.count += 1;
        if state.count > MAX_CALLS_PER_MINUTE {
            let result =
                super::protocol::CallToolResult::error("Rate limit exceeded, try again later");
            // CallToolResult is a simple struct; serialization is infallible.
            return Response::success(
                id,
                serde_json::to_value(result).expect("CallToolResult serialization"),
            );
        }
    }

    // Execute tool async with fresh DB connection and retry logic
    let result = rt.block_on(execute_tool(ctx, &params.name, params.arguments));

    // CallToolResult is a simple struct; serialization is infallible.
    Response::success(
        id,
        serde_json::to_value(result).expect("CallToolResult serialization"),
    )
}

/// Handle ping request
fn handle_ping(id: Option<Value>) -> Response {
    Response::success(id, json!({}))
}
