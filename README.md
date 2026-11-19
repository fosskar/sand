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

Releases contain a binary for Linux x86_64. It is built on Ubuntu 22.04 and needs glibc 2.34 or newer (`ldd --version` shows yours) and `libstdc++`, so it does not run on musl distributions such as Alpine. The installer downloads it from the latest GitHub release, checks it against `SHA256SUMS`, and puts `sand` into `~/.local/bin` (set `SAND_INSTALL_DIR` to change that):

```bash
curl -fsSL https://raw.githubusercontent.com/fosskar/sand/main/install.sh | sh
```

The release contains only `sand`, not the `sediment` executable. On other systems, build from source:

```bash
# From this repository
cargo install --git https://github.com/fosskar/sand

# From a checkout
cargo install --path .
```

With Nix, `nix run github:fosskar/sand` runs it, and the flake's `packages.<system>.sand` installs it.

The crates.io package `sediment-mcp` and the Homebrew formula `rendro/tap/sediment` install the original Sediment, not this repository.

## Differences from Sediment

sand 0.6.0 replaces Sediment 0.5.1 without changes to an existing setup. This was checked on a copy of a real store: Sediment 0.5.1 and sand 0.6.0 read the same store and gave identical lists, recall results and project IDs.

### Changes

- The program is named sand: the `sand` executable, the `SAND_*` settings, the `~/.sand/data` default directory, and the MCP server name.
- sand writes no `.sediment/` or `.sand/` directories and no per-repository files. In a git repository the project ID is computed from the root commit on every start. IDs of other directories live in `access.db`. Existing `.sediment/config` files are still honoured.

### Added

- `hook recall` command for `UserPromptSubmit` hooks; see [Harness integration](#harness-integration).
- Guidance in the MCP tool descriptions and server instructions on when to recall, when to store, and which scope to use.
- Nix flake with a package and a dev shell. CI runs the flake checks (build and tests, formatting, clippy) on nixbot.

### Fixed

- A note stored with scope `global` from inside a project was saved as a project note: `list --scope global` missed it, and other projects saw it as cross-project and could not forget it.
- Near-duplicates were merged, and similar notes linked, only after a recall in the MCP server, so notes stored through the CLI were rarely or never merged. Every store now starts consolidation: the MCP server runs it in the background, `sand store` runs one batch before it exits.
- Replacing a note with the MCP `store` tool did not count as a validation of the new note, while `store --replace` on the command line did. Both count it now; validations raise the trust bonus of a note in recall.
- The build failed clippy on the current Rust toolchain.

### Removed

- The Homebrew formula, the macOS binaries and the crates.io package. A GitHub Actions workflow builds the Linux x86_64 release binary again, and `install.sh` installs it; other systems install with `cargo install` or the Nix flake.
- The `init` command, which wrote usage instructions into `CLAUDE.md`. The MCP tool descriptions and the server instructions carry that guidance. Sediment and sand 0.6.0 had the command.
- The `project` command, which printed the project ID of the current directory, and the `SAND_PROJECT_ID` setting, which replaced that ID for CLI commands. sand 0.6.0 had both.
- The `consolidate` command, which drained the consolidation queue by hand. Consolidation runs after every store now. sand 0.6.0 had the command.

### Compatibility

These stay permanently:

- sand reads and writes the same database format: the LanceDB tables and `access.db` are unchanged apart from the added `projects` table. It never moves or copies the database.
- Project IDs that Sediment stored in `.sediment/config`, in the project directory or in the central state next to the database, are imported once, so the project ID stays the same. A `.sediment/` directory still marks a project root. The files are only read, never changed or deleted.

These old names are accepted until 1.0 and then removed. While sand uses one of them, it prints one line per process on stderr that names the replacement.

- A `sediment` executable is installed next to `sand` and behaves the same. MCP client configurations that start `sediment` keep working.
- Each `SAND_*` variable falls back to its `SEDIMENT_*` name when it is unset: `SEDIMENT_DB`, `SEDIMENT_EMBEDDING_MODEL`, `SEDIMENT_FTS_BOOST_MAX`, `SEDIMENT_FTS_GAMMA`. When both are set, the `SAND_*` value wins.
- Without a database path, sand uses `~/.sand/data`. If `~/.sand` does not exist and `~/.sediment/data` does, it uses `~/.sediment/data`.

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

Add to `~/.claude/settings.json`:

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

## Harness integration

`sand hook recall` adds the memories that match the first prompt of a session to the model context, so recall does not depend on the model calling the `recall` tool. It reads the `UserPromptSubmit` hook JSON of Claude Code and Codex on stdin and uses `prompt`, `cwd` (for the project) and `session_id`. On stdout it prints a `<recalled_memories>` block with at most 3 notes whose similarity before decay scoring is at least 0.5, or nothing. It recalls on the first prompt of a session and on the next ones while nothing matched, 5 prompts at most; the count per session is kept in `$XDG_RUNTIME_DIR/sand-recall` (or `/tmp/sand-recall`). `--limit`, `--min-similarity` and `--max-tries` change these values. A hook recall counts as an access for decay scoring, the same as `sand recall`.

For Claude Code, add the hook to `~/.claude/settings.json`; for Codex, the same `hooks` object goes into `~/.codex/hooks.json`:

```json
{
  "hooks": {
    "UserPromptSubmit": [
      { "hooks": [{ "type": "command", "command": "sand hook recall", "timeout": 10 }] }
    ]
  }
}
```

Harnesses without command hooks can run the same command from their own extension mechanism: pass the JSON above on stdin and add the output to the context.

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
sand stats                        # Show database statistics
sand list                         # List stored items
sand list --scope global          # List global items
sand store "some content"         # Store content
sand store -                      # Store content from stdin
sand recall "search query"        # Search by semantic similarity
sand forget <id>                  # Delete an item by ID
sand hook recall                  # Print memories for a UserPromptSubmit hook
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
