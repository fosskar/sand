# Contributing to sand

Thanks for your interest in contributing to sand!

## Getting Started

```bash
git clone https://github.com/fosskar/sand.git
cd sand
cargo build
```

`nix develop` gives a shell with the toolchain. The embedding model (~90MB) is downloaded on first use, not at build time.

## Development

```bash
# Build
cargo build

# Run tests
cargo test

# Run all tests, including the ones that need the model or git;
# point both variables at a throwaway store, never at your real one
SAND_DB=/tmp/sand-test/data SEDIMENT_DB=/tmp/sand-test/data cargo test -- --include-ignored

# Lint, with and without the bench feature
cargo clippy --all-targets -- -D warnings
cargo clippy --features bench --all-targets -- -D warnings

# Format
nix fmt

# Benchmarks
cargo bench
```

## Pull Requests

1. Fork the repo and create a feature branch
2. Make your changes
3. Ensure `nix flake check` passes (build with tests, formatting, clippy)
4. Write a clear PR description explaining the change

## Architecture

See [AGENTS.md](./AGENTS.md) for a detailed architecture overview including:
- Two-database hybrid design (LanceDB + SQLite)
- MCP server structure
- Data flow and key design decisions

## Reporting Issues

Use [GitHub Issues](https://github.com/fosskar/sand/issues).
