# AGENTS.md

Guidance for coding agents working in this repository.

This repository is `sand` (github.com/fosskar/sand). It continues Sediment 0.5.1 by rendro. The binary, the library, the crate (`sediment-mcp`), the `SEDIMENT_*` settings and the default `~/.sediment` directory still carry the old name, and so does the text below.

## Build Commands

```bash
# Build
cargo build --release

# Run MCP server (default)
cargo run --release

# Run with verbose logging
cargo run --release -- --verbose

# Run CLI commands
cargo run --release -- init                     # Initialize project
cargo run --release -- stats                    # Show database statistics
cargo run --release -- list                     # List stored items
cargo run --release -- list --scope global      # List global items
cargo run --release -- store "some content"     # Store content
cargo run --release -- store -                  # Store from stdin
cargo run --release -- recall "search query"    # Semantic search
cargo run --release -- forget <id>              # Delete an item
cargo run --release -- compact                  # Compact LanceDB storage
cargo run --release -- consolidate              # Drain the consolidation queue (merge duplicates, link related)
cargo run --release -- project                  # Print the project ID of the current directory
cargo run --release -- --json list              # JSON output (any command)

# Run tests (requires model download on first run)
cargo test
cargo test -- --ignored              # Run only the ignored tests (need the embedding model or git)
cargo test -- --include-ignored      # Run all tests

# Install locally
cargo install --path .

# Nix
nix develop                          # Dev shell: cargo, rustc, clippy, rustfmt, rust-analyzer, protobuf, treefmt
nix fmt                              # Format with treefmt (rustfmt, nixfmt)
nix flake check                      # Run the CI checks locally
nix build .#                         # Build the package (runs the tests that work offline)
```

## CI

nixbot builds the flake `checks`: `package` (build plus the tests that run without network), `formatting` (treefmt) and `clippy` (`cargo clippy --all-targets -- -D warnings`). `effects.nix` schedules a flake input update every day at 02:30. There are no GitHub workflows.

## Architecture

Sediment is a semantic memory system for AI agents, running as an MCP (Model Context Protocol) server. It combines vector search, a property graph, and access tracking into a unified memory intelligence layer.

### Two-Database Hybrid (all local, embedded, zero config)

- **LanceDB** — Vector embeddings + semantic similarity (items and chunks)
- **SQLite** (`access.db`) — Graph relationships, access tracking, decay scoring, consolidation queue

### Core Components

- **`src/main.rs`** - CLI entry point with subcommands (init, stats, list, store, recall, forget, compact, consolidate, project) and MCP server startup
- **`src/lib.rs`** - Library root exposing public API, project detection, scope types, and project ID migration
- **`src/db.rs`** - LanceDB wrapper handling vector storage, hybrid search (vector + FTS/BM25), and CRUD operations
- **`src/embedder.rs`** - Local embeddings via Candle. The `EmbeddingModel` enum lists the supported models; the default is `all-MiniLM-L6-v2` (384-dim vectors)
- **`src/chunker.rs`** - Smart content chunking by type (markdown, code, JSON, YAML, text)
- **`src/document.rs`** - ContentType enum for routing content to the appropriate chunker
- **`src/item.rs`** - Unified Item, Chunk, SearchResult, StoreResult, and ConflictInfo types
- **`src/access.rs`** - SQLite-based access tracking, validation counting, and memory decay scoring
- **`src/graph.rs`** - SQLite graph store: relationship tracking (RELATED, SUPERSEDES, CO_ACCESSED edges)
- **`src/consolidation.rs`** - Background consolidation: auto-merging near-duplicates, linking similar items
- **`src/compaction.rs`** - Background compaction of LanceDB storage: compacts fragments, prunes old versions, re-optimizes indices
- **`src/error.rs`** - SedimentError enum with typed error variants (Database, Embedding, Arrow, etc.)
- **`src/retry.rs`** - Retry utilities with exponential backoff (3 attempts, 100ms–2s)

### MCP Server (`src/mcp/`)

- **`mod.rs`** - Module exports
- **`server.rs`** - stdio JSON-RPC server with shared embedder, graph path, consolidation semaphore
- **`tools.rs`** - 4 MCP tools: `store`, `recall`, `list`, `forget`
- **`protocol.rs`** - MCP protocol types and JSON-RPC handling

### Data Flow

1. **Store**: Content → Embedder (384-dim vector) → LanceDB storage → Graph node creation → Conflict detection → Consolidation queue
2. **Chunking**: Long content (>1000 chars) → Type-aware splitting → Individual chunk embeddings
3. **Recall**: Query → Embedder → Vector similarity search → Project boosting → Decay scoring → Trust-weighted re-ranking → Graph backfill → 1-hop graph expansion → Co-access suggestions → Cross-project flagging → Background consolidation + co-access recording
4. **Consolidation** (background): Queue candidates → >=0.95 similarity: merge (delete old, transfer edges, SUPERSEDES edge) → 0.85-0.95: link (RELATED edge)
5. **Clustering** (every 10th recall): Triangle detection in graph → RELATED edges labelled `cluster-<id>`

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `SEDIMENT_DB` | `~/.sediment/data` | Override database path. Also available as `--db` CLI flag. Useful for ephemeral/isolated storage: `SEDIMENT_DB=/tmp/task sediment store "..."` |
| `SEDIMENT_EMBEDDING_MODEL` | `all-MiniLM-L6-v2` | Override embedding model. Options: `all-MiniLM-L6-v2`, `e5-small-v2`, `bge-small-en-v1.5`, `bge-base-en-v1.5` (768-dim) |
| `SEDIMENT_PROJECT_ID` | derived from the current directory | CLI only. Use this project ID instead of detecting one; an empty value means no project (global context) |
| `SEDIMENT_FTS_BOOST_MAX`, `SEDIMENT_FTS_GAMMA` | `0.12`, `2.0` | Override the BM25 boost parameters. Read only in builds with the `bench` feature |

Priority: `--db` flag > `SEDIMENT_DB` env var > default `~/.sediment/data`.

### Key Design Decisions

- **Two-database hybrid**: LanceDB for vectors, SQLite for graph relationships + mutable counters
- **Single central database** at `~/.sediment/data/` stores all projects; graph + access at `~/.sediment/access.db`
- **Project scoping**: in a git repository the project ID is the root commit hash, so clones and worktrees share it. Its config is cached under `<parent of the database path>/projects/<sha256 of the repository path>/.sediment/config`. A directory outside git gets a UUID in `<root>/.sediment/config`
- **Similarity boosting**: Same-project items unchanged, different projects get 0.875x penalty (12.5% spread)
- **Hybrid search**: Vector similarity combined with FTS/BM25 scoring. BM25 boost is additive (max 0.12, power-law gamma 2.0). FTS index rebuilt on each store
- **Conflict detection**: Items with >=0.85 similarity flagged on store and enqueued for consolidation
- **Fresh DB connection per tool call** with shared embedder for efficiency
- **Memory decay scoring**: Recall results re-ranked using freshness (hyperbolic decay, 0.5 at 30 days) and access frequency (log-scaled). Tracked in SQLite sidecar since LanceDB is append-oriented.
- **Trust-weighted scoring**: `final_score = similarity * freshness * frequency * trust_bonus` where `trust_bonus = 1.0 + 0.05*ln(1+validation_count) + 0.005*edge_count`
- **Non-blocking intelligence**: All background tasks (consolidation, co-access tracking, clustering) run as fire-and-forget `tokio::spawn` tasks. Tool responses return immediately. `Semaphore(1)` prevents concurrent consolidation.
- **Lazy graph backfill**: Pre-existing items get graph nodes when they appear in recall results
- **Auto-migration**: Database schema is automatically migrated on startup when upgrading from older versions

## MCP Tools Reference

The 4-tool API is defined in `src/mcp/tools.rs`:

| Tool | Purpose |
|------|---------|
| `store` | Store content with optional scope (project/global) |
| `recall` | Semantic search with decay scoring, trust weighting, graph expansion |
| `list` | List items by scope (project/global/all) |
| `forget` | Delete item by ID (removes from LanceDB and graph) |

### Store Parameters
- `content` (required) — The content to store
- `scope` (optional, default: "project") — Where to store: "project" or "global"
- `replace_id` (optional) — ID of an existing item to replace. Atomically stores new content and deletes the old item, preserving graph lineage.

### Recall Parameters
- `query` (required) — Semantic search query
- `limit` (optional, default: 5) — Maximum number of results

### List Parameters
- `limit` (optional, default: 10) — Maximum number of results
- `scope` (optional, default: "project") — Which items to list: "project", "global", or "all"

### Recall Response Fields
- `results[]` — standard results with `similarity`, optional `raw_similarity` (score before decay and trust weighting), `related_ids`, optional `cross_project` flag
- `graph_expanded[]` — 1-hop neighbors from graph not in original results (marked `graph_expanded: true`)
- `suggested[]` — items frequently co-recalled with top results (co-access count >= 3)

## SQLite Schema (access.db)

```sql
-- Graph nodes
CREATE TABLE graph_nodes (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL
);

-- Graph edges (RELATED, SUPERSEDES, CO_ACCESSED)
CREATE TABLE graph_edges (
    from_id TEXT NOT NULL,
    to_id TEXT NOT NULL,
    edge_type TEXT NOT NULL,     -- 'related', 'supersedes', 'co_accessed'
    strength REAL NOT NULL DEFAULT 0.0,
    rel_type TEXT NOT NULL DEFAULT '',
    count INTEGER NOT NULL DEFAULT 0,
    last_at INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    UNIQUE(from_id, to_id, edge_type)
);
CREATE INDEX idx_edges_from ON graph_edges(from_id, edge_type);
CREATE INDEX idx_edges_to ON graph_edges(to_id, edge_type);

-- Access tracking and decay scoring
CREATE TABLE access_log (
    item_id TEXT PRIMARY KEY,
    access_count INTEGER NOT NULL DEFAULT 0,
    last_accessed_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    validation_count INTEGER NOT NULL DEFAULT 0  -- added by migration; incremented on replace
);

-- Consolidation queue (populated on store conflicts)
CREATE TABLE consolidation_queue (
    item_id_a TEXT NOT NULL, item_id_b TEXT NOT NULL, similarity REAL NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',  -- pending/merged/linked
    created_at INTEGER NOT NULL,
    UNIQUE(item_id_a, item_id_b)
);
```

## Testing Notes

- Embedder tests are marked `#[ignore]` because they require downloading the model (~90MB). The other ignored tests (in `db.rs`, `lib.rs`, `mcp/tools.rs` and `tests/`) need the model or a git repository; 33 tests are ignored in total and none of them run in the Nix build
- Set `SEDIMENT_DB` to a temporary path when you run the ignored tests or the binary by hand. Project configs are written next to the database path, and the default is the real store in `~/.sediment`
- `tests/cli_global_scope.rs` is an integration test that runs the built binary
- Use `tempfile` crate for database tests to avoid polluting real data
- LanceDB operations are async; tests use `#[tokio::test]` or `tokio::runtime::Runtime`
