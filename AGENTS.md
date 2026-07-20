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
- `docs/superpowers/specs/2026-07-21-mcp-agent-interface-design.md` — MCP tools,
  flush-on-close, capabilities discovery.
- `skills/xlsx/SKILL.md` — agent workflow for `.xlsx` via Dotall MCP.

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

```text
dotall/
├── Cargo.toml               # [workspace]
├── crates/
│   ├── dotall-core/         # store, registry, pipeline, read, history, orchestration
│   ├── dotall-xlsx/         # typed model, processors, views, edits, OOXML writer
│   ├── dotall-cli/          # bin: `dotall`
│   └── dotall-mcp/          # bin: stdio MCP server (`dotall-mcp`)
```

Core concerns begin as strict internal modules. Promote one to a separate crate only
when independent dependencies, feature gating, test isolation, or ownership make
the boundary valuable. Do not generalize a universal model or graph from XLSX
alone.

The format handler contract is the plug-and-play boundary. Shared envelopes carry
versioned, format-owned payloads; normal agent workflows never see those internal
schemas:

```rust
trait FormatHandler {
    fn detect(&self, probe: &DetectionProbe) -> DetectionScore;
    fn capabilities(&self) -> Capabilities;
    fn parse(&self, source: &Path) -> Result<ArtifactEnvelope>;
    fn read(&self, model: &ArtifactEnvelope, request: &ReadRequest)
        -> Result<ReadResponse>;
    fn validate_edit(&self, model: &ArtifactEnvelope, ops: &[SemanticOperation])
        -> Result<ValidatedEdit>;
    fn apply_edit(&self, source: &Path, edit: &ValidatedEdit)
        -> Result<PatchedOutput>;
}
```

## `.all/` layout

```text
.all/
├── manifest.json
└── objects/<file>/
    ├── meta.json            # mime, size, mtime, source hash, schema ver
    ├── original.ref         # pointer to source (no copy)
    ├── cache/               # safe to delete and regenerate
    │   ├── model/
    │   ├── derived/
    │   └── views/
    └── state/               # retained across cache invalidation
        ├── access/log.jsonl
        ├── transactions/
        └── edits/
            ├── staging/
            └── history/ + snapshots/
```

`.all/` is gitignored runtime state. `cache/` is regenerable; `state/` contains
durable local history and recovery data and must not be removed by cache cleanup.

## Conventions

- **Language:** Rust (stable). One Cargo workspace; focused modules inside core and
  one crate per format family.
- **Errors:** `Result` everywhere; no `unwrap()` in library code outside tests.
- **Serialization:** `serde` + `serde_json`. Models/views are JSON for now
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

## MCP (agents)

Stdio MCP server over the same `Engine` as the CLI. Run from the workspace root:

```bash
cargo run -p dotall-mcp
```

Register the built binary in your MCP client. Before editing `.xlsx` files, read
`skills/xlsx/SKILL.md` — discover capabilities at runtime, stage edits, apply
explicitly or via flush-on-close (default).

## Build & test

```bash
cargo build
cargo test
cargo run -p dotall-cli -- <args>
cargo run -p dotall-mcp
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
