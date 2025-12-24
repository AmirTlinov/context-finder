# Context Finder MCP

Semantic code search **built for AI agents**: index once, then ask for **one bounded context pack** you can feed into a model or pipeline.

If you’re tired of “search → open file → search again → maybe the right function?”, Context Finder turns a query into a compact, contract-stable JSON response — with optional graph-aware “halo” context.

## What you get

- **Agent-first output:** `context-pack` returns a single JSON payload bounded by `max_chars`.
- **One-call orchestration:** `action=batch` runs multiple actions under one `max_chars` budget (partial success per item).
- **Safe file reads:** MCP `file_slice` returns a bounded file window (root-locked, line-based, hashed).
- **Regex context reads:** MCP `grep_context` returns all regex matches with `before/after` context (grep `-B/-A/-C`), merged into compact hunks under hard budgets.
- **Safe file listing:** MCP `list_files` returns bounded file paths (glob/substring filter).
- **Repo onboarding pack:** MCP `repo_onboarding_pack` returns `map` + key docs (`file_slice`) + `next_actions` in one bounded response.
- **One-call reading pack:** MCP `read_pack` picks the right tool (`file_slice` / `grep_context` / `context_pack` / `repo_onboarding_pack`) and returns `sections` + `next_actions` under one `max_chars` budget.
- **Cursor pagination:** `map`, `list_files`, `text_search`, `grep_context`, `file_slice` return `next_cursor` when truncated so agents can continue without guessing.
- **Freshness by default:** every response can carry `meta.index_state`; `options.stale_policy=auto|warn|fail` controls (re)index behavior.
- **Stable integration surfaces:** CLI JSON, HTTP, gRPC, MCP — all treated as contracts.
- **Hybrid retrieval:** semantic + fuzzy + fusion + profile-driven boosts.
- **Graph-aware context:** attach related chunks (calls/imports/tests) when you need it.
- **Task packs:** `task_pack` adds `why` + `next_actions` on top of `context_pack`.
- **Bounded text search:** `text_search` uses corpus when present and can fall back to filesystem scanning safely.
- **Measured quality:** golden datasets + MRR/recall/latency/bytes + A/B comparisons.
- **Offline-first models:** download once from a manifest, verify sha256, never commit assets.
- **No silent CPU fallback:** CUDA by default; CPU only if explicitly allowed.

## 60-second quick start

### 1) Build and install

```bash
git clone https://github.com/AmirTlinov/context-finder-mcp.git
cd context-finder-mcp

cargo build --release
cargo install --path crates/cli
```

Optional local alias (avoids `cargo install` during iteration):

```bash
alias context-finder='./target/release/context-finder'
```

### 2) Install models (offline) and verify

Model assets are downloaded once into `./models/` (gitignored) from `models/manifest.json`:

```bash
context-finder install-models
context-finder doctor --json
```

Execution policy:

- GPU-only by default (CUDA).
- CPU fallback is allowed only when `CONTEXT_FINDER_ALLOW_CPU=1`.

### 3) Index and ask for a bounded pack

```bash
cd /path/to/project

context-finder index . --json
context-finder context-pack "index schema version" --path . --max-chars 20000 --json --quiet
```

Want exploration with graph expansion?

```bash
context-finder context "streaming indexer health" --path . --strategy deep --show-graph --json --quiet
```

## Integrations

### CLI + JSON Command API

One request shape; one response envelope:

```bash
context-finder command --json '{"action":"search","payload":{"query":"embedding templates","limit":5,"project":"."}}'
```

Task-oriented pack with freshness guard and path filters:

```bash
context-finder command --json '{
  "action":"task_pack",
  "options":{"stale_policy":"auto","max_reindex_ms":1500,"include_paths":["src"]},
  "payload":{"intent":"refresh watermark policy","project":".","max_chars":20000}
}'
```

Batch (one request → many actions):

```bash
context-finder command --json '{
  "action":"batch",
  "options":{"stale_policy":"auto","max_reindex_ms":1500},
  "payload":{
    "project":".",
    "max_chars":20000,
    "items":[
      {"id":"idx","action":"index","payload":{"path":"."}},
      {"id":"pack","action":"context_pack","payload":{"query":"stale policy gate","limit":6}}
    ]
  }
}'
```

### HTTP

```bash
context-finder serve-http --bind 127.0.0.1:7700
```

- `POST /command`
- `GET /health`

### gRPC

```bash
context-finder serve-grpc --bind 127.0.0.1:50051
```

### MCP server

```bash
cargo install --path crates/mcp-server
```

Example Codex config (`~/.codex/config.toml`):

```toml
[mcp_servers.context-finder]
command = "context-finder-mcp"
args = []

[mcp_servers.context-finder.env]
CONTEXT_FINDER_PROFILE = "quality"
```

Fastest way to orient on a new repo (one MCP call → map + key docs + next actions): use `repo_onboarding_pack`:

```jsonc
{
  "path": "/path/to/project",
  "map_depth": 2,
  "docs_limit": 6,
  "max_chars": 20000
}
```

Want one MCP tool to replace `cat`/`sed`, `rg -C`, *and* semantic packs? Use `read_pack`:

```jsonc
// Read a file window (file_slice)
{
  "path": "/path/to/project",
  "intent": "file",
  "file": "src/lib.rs",
  "start_line": 120,
  "max_lines": 80,
  "max_chars": 20000
}

// Continue without repeating inputs (cursor-only continuation)
{
  "path": "/path/to/project",
  "cursor": "<next_cursor>"
}
```

Need grep-like reads with N lines of context across a repo (without `rg` + `sed` loops)? Use `grep_context`:

```jsonc
{
  "path": "/path/to/project",
  "pattern": "stale_policy",
  "file_pattern": "crates/*/src/*",
  "before": 50,
  "after": 50,
  "max_hunks": 40,
  "max_chars": 20000
}
```

If the output is truncated, the response includes `next_cursor`. Call again with the same options + `cursor: "<next_cursor>"`.

Agent-friendly tip: the MCP tool `batch` lets you execute multiple tools in one call (one bounded JSON result). In batch `version: 2`, item inputs can depend on earlier outputs via `$ref` (JSON Pointer):

```jsonc
{
  "version": 2,
  "path": "/path/to/project",
  "max_chars": 20000,
  "items": [
    { "id": "hits", "tool": "text_search", "input": { "pattern": "stale_policy", "max_results": 1 } },
    {
      "id": "ctx",
      "tool": "grep_context",
      "input": {
        "pattern": "stale_policy",
        "file": { "$ref": "#/items/hits/data/matches/0/file" },
        "before": 40,
        "after": 40
      }
    }
  ]
}
```

When you need the *exact* contents of a file region (without `cat`/`sed`), use the MCP tool `file_slice`:

```jsonc
{
  "path": "/path/to/project",
  "file": "src/lib.rs",
  "start_line": 120,
  "max_lines": 80,
  "max_chars": 8000
}
```

If the response is truncated, continue with `cursor` (keep the same limits):

```jsonc
{
  "path": "/path/to/project",
  "file": "src/lib.rs",
  "cursor": "<next_cursor>",
  "max_lines": 80,
  "max_chars": 8000
}
```

When you need file paths first (without `ls/find/rg --files`), use `list_files`:

```jsonc
{
  "path": "/path/to/project",
  "file_pattern": "src/*",
  "limit": 200,
  "max_chars": 8000
}
```

## Contracts (source of truth)

All integration surfaces are contract-first and versioned:

- [contracts/README.md](contracts/README.md)
- [contracts/command/v1/](contracts/command/v1/) (JSON Schemas)
- [contracts/http/v1/openapi.json](contracts/http/v1/openapi.json) (OpenAPI 3.1)
- [proto/](proto/) (gRPC)

## Documentation

- [docs/README.md](docs/README.md) (navigation hub)
- [docs/QUICK_START.md](docs/QUICK_START.md) (install, CLI, servers, JSON API)
- [USAGE_EXAMPLES.md](USAGE_EXAMPLES.md) (agent-first workflows)
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- [docs/CONTEXT_PACK.md](docs/CONTEXT_PACK.md)
- [docs/COMMAND_RFC.md](docs/COMMAND_RFC.md)
- [PHILOSOPHY.md](PHILOSOPHY.md)
- [models/README.md](models/README.md)
- [bench/README.md](bench/README.md)

## Development

```bash
scripts/validate_contracts.sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
CONTEXT_FINDER_EMBEDDING_MODE=stub cargo test --workspace
```

## License

MIT OR Apache-2.0

## Contributing

See `CONTRIBUTING.md`.
