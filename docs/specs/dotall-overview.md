# Dotall — Product & Architecture Overview

## Thesis

Files were built for humans. Agents get a lossy snapshot every time they read one,
start from zero each new session, and have no version history when they edit. This
isn't a prompting problem — it's a missing filesystem primitive.

**Dotall is the translation layer between agents and human files.** Every project has
`.git/`. Every agent project needs `.all/` — a hidden directory that holds cached
reads, versioned edits, and agent-readable projections of every file.

## What `.all/` is

Analogous to `.git/`, but for **agent cognition + cached access + edit versions**.

- **Access cache (read path):** on first read we parse the file into a structured
  model and materialize agent-readable projections. Subsequent reads are cache hits —
  no reparse, minimal tokens.
- **Edit versioning (write path):** every agent edit is staged, applied via a
  fidelity-preserving round-trip, and recorded as an immutable version with a
  semantic diff and revert path. Originals stay untouched until apply.

## The four layers

```
Human file (bytes)
      ↓ parse
① Syntactic tree (AST)     — structure: cells, paragraphs, form fields
      ↓ analyze
② Semantic graph           — meaning: deps, refs, entities, roles
      ↓ project
③ Agent view               — token-efficient, LLM-friendly context
      ↓ edit
④ Version history          — staged ops → applied → v001, v002…
      ↓ round-trip
Human file (bytes, preserved)
```

- **① AST** — lossless-ish, format-specific structure with stable element IDs.
  Round-trip capable. Built once on cold read, never rebuilt on cache hit.
- **② Semantic graph** — relationships syntax alone doesn't expose (e.g. cell B12
  depends on A1:A10). Enables meaningful edits and semantic diffs.
- **③ Agent view** — progressive disclosure (summary → range → subgraph → full) plus
  a directly-readable Markdown projection. This is the token-savings USP.
- **④ Version history** — git-like `history` / `diff` / `revert` over agent edits.

## Stack

| Layer | Technology | Role |
|-------|------------|------|
| Core | **Rust** | AST, semantics, cache, views, edit/version pipeline |
| CLI | **Rust** (`dotall`) | Dev harness + real CLI over the core |
| MCP | **Rust** (stdio binary) | The interface agents use in Claude Desktop |
| SDKs | Python, JS (later) | Thin bindings over the Rust core |
| Storage | `.all/` | Syntactic + semantic + views + edits |

Distribution target: a **single static Rust binary**. Install = point Claude
Desktop's MCP config at the binary — no Python/Node runtime required.

## Modular, plug-and-play

Each concern is an isolated crate behind a narrow interface. Formats implement a
`Format` trait and register in a format registry — adding DOCX or PDF is a new crate,
not a core rewrite. This serves both performance (swap implementations behind stable
interfaces) and maintainability (reason about and test one unit at a time).

## Roadmap

| Phase | Formats | Notes |
|-------|---------|-------|
| **1 — Office** | XLSX → DOCX → PDF | XLSX first: formula preservation is the moat |
| 2 — Media | Images, audio | Layers/OCR/EXIF; transcript/timeline graphs |
| 3 — Scientific | EEG/MEG (MNE), etc. | Channel montages, epoch relationships |
| 4 — CAD | STEP, DWG, STL | Geometry/assembly graphs |

Same `.all/` envelope, different AST modules per format.

### Why Office first

Most agent workflows hit Excel/Word/PDF; the buyers (legal, finance, ops) have
budget; DOCX/XLSX are XML with real ASTs; and "preserve the formulas" is a value
moment a generic text dump can't match. It proves the pipeline — everything else
plugs in.

## Current milestone

XLSX engine, CLI-first, pure Rust. See `docs/specs/xlsx-engine-v0.md`.

## Key architecture decisions

- **`.all/` scope:** project-local for v0; global content-addressed cache deferred.
- **Object keying:** relative path primary; content hash for invalidation/dedup.
- **Original storage:** reference by default (no copy); snapshots only when a version
  needs a revert target.
- **Invalidation:** `blake3` + `(mtime, size)` fast path; `status`/`sync` to rescan;
  `watch` daemon deferred.
- **Edit flow:** auto-apply by default (agents), `--stage-only` for human review.
- **Round-trip:** surgical OOXML patching to preserve untouched content.

## Non-goals (v0)

No formula evaluation engine · no global cache · no `watch` daemon · no multi-agent
conflict handling · no branching history · no DOCX/PDF yet · no Python/JS SDK yet ·
no editing of charts/pivots (preserve, don't mutate).
