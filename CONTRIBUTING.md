# Contributing

Thanks for your interest in contributing to Context Finder.

## Development setup

Requirements:

- Rust (stable)
- A working C/C++ toolchain for native deps (standard Rust setup)

Model assets are optional for most development tasks. For deterministic, model-free tests:

```bash
CONTEXT_FINDER_EMBEDDING_MODE=stub cargo test --workspace
```

## Quality gates

Run these before opening a PR:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
CONTEXT_FINDER_EMBEDDING_MODE=stub cargo test --workspace
```

## Documentation

- Documentation is maintained in English (`*.md`).
- Keep command examples consistent with `context-finder --help`.

## Models and caches

- Do not commit downloaded model assets under `models/**`.
- Do not commit local caches (`.context-finder/`, `.fastembed_cache/`, `.deps/`, etc.).

## Benchmarks and datasets

- Bench harness lives under `bench/` (see `bench/README.md`).
- Golden evaluation datasets live under `datasets/`.

## PR hygiene

- Keep changes focused and avoid unrelated refactors.
- Add tests for behavior changes.
- Prefer clear error messages and stable JSON outputs.

