# sand

Semantic memory for AI agents. Local-first, MCP-native.

sand is based on [Sediment](https://github.com/rendro/sediment) by [rendro](https://github.com/rendro), who designed and wrote the program described below. This repository starts from Sediment 0.5.1 and is maintained independently; it is not affiliated with or endorsed by the original author. Changes made here are listed in the commit history after the `Bump version to 0.5.1` commit.

The text below is adapted from Sediment's documentation; the benchmark numbers in it were measured by the original author on Sediment.

Combines vector search, a relationship graph, and access tracking into a unified memory intelligence layer — all running locally as a single binary.

## Why sand?

- **Single binary, zero config** — no Docker, no Postgres, no Qdrant. Just `sand`.
- **50ms store, 103ms recall** — local embeddings and vector search at 1K items, no network round-trips.
- **4-tool focused API** — `store`, `recall`, `list`, `forget`. That's it.
- **Works everywhere** — macOS (Intel + ARM), Linux x86_64. All data stays on your machine.

### Comparison

| | Sediment | ChromaDB | Mem0 | MCP Memory Svc |
|---|---|---|---|---|
| Install | Single binary | Python + pip | Docker + Postgres + Qdrant | Python + pip |
| Tools | 4 | API | 4 | 12 |
| R@1 | **50%** | 47% | 47% | 38% |
| Recency@1 | **100%** | 14% | 14% | 10% |
| Dedup | **99%** | 0% | 0% | 0% |
| Store p50 | 50ms | 692ms | 14ms | 2ms |

## Install

```bash
# From this repository
cargo install --git https://github.com/fosskar/sand

# From a checkout
cargo install --path .
```

The crates.io package `sediment-mcp` and the Homebrew formula `rendro/tap/sediment` install the original Sediment, not this repository.

## Differences from Sediment

sand 0.6.0 replaces Sediment 0.5.1 without changes to an existing setup.

### Changes

- The program is named sand: the `sand` executable, the `SAND_*` settings, the `~/.sand/data` default directory, the MCP server name, and the `mcp__sand__` instructions that `sand init` writes.
- sand writes no `.sediment/` or `.sand/` directories and no per-repository files. In a git repository the project ID is computed from the root commit on every start. IDs of other directories live in `access.db`. Existing `.sediment/config` files are still honoured.

### Compatibility

These stay permanently:

- sand reads and writes the same database format: the LanceDB tables and `access.db` are unchanged apart from the added `projects` table. It never moves or copies the database.
- Project IDs that Sediment stored in `.sediment/config`, in the project directory or in the central state next to the database, are imported once, so the project ID stays the same. A `.sediment/` directory still marks a project root. The files are only read, never changed or deleted.

These old names are accepted until 1.0 and then removed. While sand uses one of the first three, it prints one line per process on stderr that names the replacement.

- A `sediment` executable is installed next to `sand` and behaves the same. MCP client configurations that start `sediment` keep working.
- Each `SAND_*` variable falls back to its `SEDIMENT_*` name when it is unset: `SEDIMENT_DB`, `SEDIMENT_EMBEDDING_MODEL`, `SEDIMENT_PROJECT_ID`, `SEDIMENT_FTS_BOOST_MAX`, `SEDIMENT_FTS_GAMMA`. When both are set, the `SAND_*` value wins.
- Without a database path, sand uses `~/.sand/data`. If `~/.sand` does not exist and `~/.sediment/data` does, it uses `~/.sediment/data`.
- `sand init` recognises the instructions Sediment wrote (`mcp__sediment__`) and does not add a second block.

One direction is not covered: a directory outside git that was first used with sand has no `.sediment/config`, so going back to Sediment gives it a new project ID there.

## Setup

Add sand to your MCP client configuration:

### Claude Desktop

Add to `~/Library/Application Support/Claude/claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "sand": {
      "command": "sand"
    }
  }
}
```

### Claude Code

Run `sand init` in your project, or add manually to `~/.claude/settings.json`:

```json
{
  "mcpServers": {
    "sand": {
      "command": "sand"
    }
  }
}
```

### Cursor

Add to `.cursor/mcp.json` in your project:

```json
{
  "mcpServers": {
    "sand": {
      "command": "sand"
    }
  }
}
```

### VS Code (Copilot)

Add to `.vscode/mcp.json` in your project:

```json
{
  "servers": {
    "sand": {
      "command": "sand"
    }
  }
}
```

### Windsurf

Add to `~/.codeium/windsurf/mcp_config.json`:

```json
{
  "mcpServers": {
    "sand": {
      "command": "sand"
    }
  }
}
```

### JetBrains IDEs

Go to **Settings > Tools > AI Assistant > MCP Servers**, click **+**, and add:

```json
{
  "sand": {
    "command": "sand"
  }
}
```

### Zed

Add to `~/.zed/settings.json` on macOS or `~/.config/zed/settings.json` on Linux:

```json
{
  "context_servers": {
    "sand": {
      "command": "sand",
      "args": [],
    }
  }
}
```

## Tools

| Tool | Parameters | Description |
|------|------------|-------------|
| `store` | `content`, `scope?`, `replace_id?` | Save content to memory |
| `recall` | `query`, `limit?` | Search by semantic similarity |
| `list` | `limit?`, `scope?` | List stored items |
| `forget` | `id` | Delete an item by ID |

## CLI

```bash
sand                              # Start MCP server
sand init                         # Set up Claude Code integration
sand stats                        # Show database statistics
sand list                         # List stored items
sand list --scope global          # List global items
sand store "some content"         # Store content
sand store -                      # Store content from stdin
sand recall "search query"        # Search by semantic similarity
sand forget <id>                  # Delete an item by ID
```

All commands support `--json` for machine-readable output.

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `SAND_DB` | `~/.sand/data` | Override database path. Useful for ephemeral or isolated storage. |
| `SAND_EMBEDDING_MODEL` | `all-MiniLM-L6-v2` | Override embedding model (`all-MiniLM-L6-v2`, `bge-small-en-v1.5`). |

Example — use a throwaway database for a task:

```bash
SAND_DB=/tmp/task-xyz sand store "temporary context"
SAND_DB=/tmp/task-xyz sand recall "context"
```

The `--db` CLI flag takes the same value and overrides the env var if both are set.

## How It Works

### Two-Database Hybrid

All local, embedded, zero config:

- **LanceDB** — Vector embeddings and semantic similarity search
- **SQLite** (`access.db`) — Relationship graph, access tracking, decay scoring, consolidation queue

### Key Features

- **Memory decay**: Results re-ranked by freshness (30-day half-life) and access frequency. Old memories rank lower but are never auto-deleted.
- **Trust-weighted scoring**: Validated and well-connected memories score higher.
- **Hybrid search**: Vector similarity combined with FTS/BM25 scoring for better retrieval quality.
- **Project scoping**: Automatic context isolation between projects. Different-project items receive a similarity penalty.
- **Relationship graph**: Items linked via RELATED, SUPERSEDES, and CO_ACCESSED edges. Recall expands results with 1-hop graph neighbors and co-access suggestions.
- **Background consolidation**: Near-duplicates (≥0.95 similarity) auto-merged; similar items (0.85–0.95) linked.
- **Type-aware chunking**: Intelligent splitting for markdown, code, JSON, YAML, and plain text.
- **Conflict detection**: Items with ≥0.85 similarity flagged on store.
- **Cross-project recall**: Results from other projects flagged.
- **Local embeddings**: all-MiniLM-L6-v2 via Candle (384-dim vectors, no API keys).
- **Model integrity**: SHA-256 verification of all model files on every load, pinned to a specific revision.
- **Auto-migration**: Database schema automatically migrated when upgrading from older versions.

### Security

- **Input bounds**: Content (1MB), queries (10KB), JSON-RPC lines (10MB).
- **Rate limiting**: 600 tool calls per minute.
- **SQL injection prevention**: Sanitized filter expressions for LanceDB; parameterized queries for SQLite.
- **Cross-project access control**: Forget enforces project isolation. Cross-project content is flagged in recall results.
- **Error sanitization**: Internal errors logged to stderr; only generic messages returned to MCP clients.
- **Retry with backoff**: Transient failures retried with exponential backoff (3 attempts, 100ms–2s).

## Performance

Benchmarked against 5 alternatives with 1,000 memories and 200 queries. See [BENCHMARKS.md](BENCHMARKS.md) for full results.

| Metric | Sediment | ChromaDB | Mem0 |
|--------|----------|----------|------|
| R@1 | **50%** | 47% | 47% |
| MRR | **62%** | 61% | 61% |
| Recency@1 | **100%** | 14% | 14% |
| Dedup | **99%** | 0% | 0% |
| Store p50 | 50ms | 692ms | 14ms |
| Recall p50 | 103ms | 694ms | 8ms |

### Data Location

- Vector store: `~/.sand/data/`
- Graph + access tracking: `~/.sand/access.db`

Everything runs locally. Your data never leaves your machine.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for build instructions and PR guidelines.

## License

[MIT](LICENSE)
