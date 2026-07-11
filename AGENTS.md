# Dotall — Agent Instructions

Dotall is the **translation layer between AI agents and human files**. It caches
every read, versions every edit, and exposes files to LLMs through structured,
token-efficient, agent-readable projections — all stored in a hidden `.all/`
directory, analogous to `.git/`.

This repository is the **Rust engine** (core + CLI + MCP). The marketing site and
brainstorming notes live elsewhere.

## Read first

- `docs/specs/dotall-overview.md` — product thesis, architecture, `.all/` model.
- `docs/specs/xlsx-engine-v0.md` — the current milestone: XLSX engine, CLI-first.

Specs are the source of truth. If code and spec disagree, fix one deliberately —
don't silently drift.

## Core principles

1. **Agent-readability first.** Humans use Excel/Word/etc. for the original file;
   the artifacts we generate are optimized for LLM consumption.
2. **Modular, plug-and-play.** Every concern is an isolated crate behind a narrow
   interface. Formats plug in via the `Format` trait — new formats are new crates,
   never core rewrites. Keep units small and independently testable; you should be
   able to swap an implementation without touching consumers.
3. **Own the pipeline, integrate the parsers.** We own the IR, cache, semantic
   graph, views, and edit/version pipeline. Integrate existing parsers first;
   replace format-by-format as the moat deepens.
4. **YAGNI.** Build the thin vertical slice, then broaden. Respect the non-goals in
   the spec — don't build ahead of them without a decision.

## Architecture (target)

```
dotall/
├── Cargo.toml               # [workspace]
├── crates/
│   ├── dotall-core/         # orchestrates the crates below
│   ├── dotall-cache/        # .all/ IO, manifest, hashing, invalidation
│   ├── dotall-model/        # canonical AST types + JSON, stable element IDs
│   ├── dotall-semantics/    # formula dependency graph
│   ├── dotall-views/        # L0 summary + Markdown projection + range slices
│   ├── dotall-edit/         # op types, staging, apply, version/diff/revert
│   ├── dotall-xlsx/         # format module: read + surgical OOXML write
│   ├── dotall-cli/          # bin: `dotall`
│   └── dotall-mcp/          # bin: stdio MCP server (added later)
```

The `Format` trait is the plug-and-play boundary:

```rust
trait Format {
    fn parse(&self, bytes: &[u8]) -> Result<Model>;
    fn summary(&self, model: &Model) -> Summary;
    fn apply_ops(&self, source: &Path, ops: &[EditOp]) -> Result<Vec<u8>>;
}
```

## `.all/` layout

```
.all/
├── manifest.json
└── objects/<file>/
    ├── meta.json            # mime, size, mtime, source hash, schema ver
    ├── original.ref         # pointer to source (no copy)
    ├── ast/model.json       # canonical AST (source of truth for edits)
    ├── graph/deps.json      # formula dependency graph
    ├── views/
    │   ├── summary.json      # L0 summary
    │   └── sheets/<sheet>.md # LLM-friendly Markdown projection
    ├── access/log.jsonl     # read provenance
    └── edits/
        ├── staging/patch-*.json
        └── history/vNNN.json + snapshots/vNNN.ref
```

`.all/` is gitignored (regenerable cache).

## Conventions

- **Language:** Rust (stable). One Cargo workspace; one concern per crate.
- **Errors:** `Result` everywhere; no `unwrap()` in library code outside tests.
- **Serialization:** `serde` + `serde_json`. AST/views are JSON for now
  (agent- and human-debuggable); optimize to binary only if profiling demands it.
- **Hashing:** `blake3` with an `(mtime, size, hash)` fast path — never rehash large
  files on every call.
- **Formatting/lint:** `cargo fmt` and `cargo clippy` clean before done.
- **Naming:** crates `dotall-*`; snake_case modules; stable `element_id`s for any
  node an edit can target.

## Round-trip fidelity (the crux)

Edits use **surgical OOXML patching**, not full re-serialization: patch only the
target parts in the `.xlsx` ZIP and leave every untouched part byte-for-byte. This is
the moat — "agents break spreadsheets, we don't." Guard it with golden round-trip
tests.

## Build & test

```
cargo build
cargo test
cargo run -p dotall-cli -- <args>
cargo fmt && cargo clippy
```

## Testing expectations

- Round-trip golden tests on a real `.xlsx` corpus (formulas, styles, charts,
  multi-sheet, named ranges): untouched parts stay byte-identical; edits are correct.
- Unit tests for the formula-ref graph, cache hit/miss + invalidation, snapshot/revert.

## Non-goals (v0)

No formula evaluation engine · no global `~/.all/cache` · no `watch` daemon · no
multi-agent conflict/locking · no branching history · no DOCX/PDF · no Python/JS SDK
· no charts/pivots **editing** (preserve them, don't mutate).

## Git

- `main` is the default branch.
- Never commit `.all/`, `/target/`, or secrets.
- Only commit when the user asks.
