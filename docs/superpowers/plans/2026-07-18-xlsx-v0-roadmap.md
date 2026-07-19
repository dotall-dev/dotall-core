# XLSX v0 Plan Roadmap

> **For agentic workers:** Execute each linked plan with superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Reconcile later plans with actual APIs after each completed slice.

**Goal:** Deliver the XLSX-first Dotall vertical slice from project-local cache through safe agent reads, semantic dependencies, transactional edits, broader spreadsheet operations, and MCP.

**Architecture:** Each plan produces independently testable working software and establishes contracts consumed by the next plan. Core remains one modular crate; XLSX remains one format-family crate; CLI and MCP compose formats without introducing concrete-format dependencies into core.

**Tech Stack:** Stable Rust workspace, project-local `.all/`, BLAKE3, serde JSON artifacts, calamine, quick-xml/zip surgical writing, Clap, and the official rmcp Rust SDK.

---

## Execution order

1. [`2026-07-18-core-storage-foundation.md`](2026-07-18-core-storage-foundation.md)
   - Workspace, `.all/`, atomic manifest, source fingerprints, registration,
     `init`, and `status`.
   - Exit gate: cold state is durable; status distinguishes fresh/stale/missing.

2. [`2026-07-19-xlsx-read-pipeline.md`](2026-07-19-xlsx-read-pipeline.md)
   (supersedes `2026-07-18-xlsx-read-pipeline.md`; AST design:
   `docs/superpowers/specs/2026-07-19-xlsx-translation-ast-design.md`)
   - Format contracts, translation AST (`xlsx.workbook` v1), dual dialects,
     cached inspection/read, ranges, budgets, and continuations.
   - Exit gate: second inspection does not reparse or rewrite the model artifact.

3. [`2026-07-18-xlsx-formula-dependencies.md`](2026-07-18-xlsx-formula-dependencies.md)
   - Formula reference lexer, dependency graph, derivation recipes, cache, and
     forward/reverse agent queries.
   - **Prerequisite for writes** per
     `docs/superpowers/specs/2026-07-20-xlsx-write-history-design.md`.
   - Exit gate: identical query is a derived-cache hit; no formula evaluation.

4. [`2026-07-18-transactional-xlsx-edits.md`](2026-07-18-transactional-xlsx-edits.md)
   - Merge 1 writes: `set_cell_value`, `set_cell_formula`, surgical writer,
     stage/apply, optimistic concurrency, idempotency, journals, snapshots,
     forensic history, diff, recovery, and revert.
   - Revise against `2026-07-20-xlsx-write-history-design.md` (stage-default,
     MCP close-flush) before execution.
   - Exit gate: targeted edits are correct, untouched OOXML parts are byte-identical,
     and revert restores the expected state as a new version.

5. [`2026-07-18-broaden-xlsx-edit-operations.md`](2026-07-18-broaden-xlsx-edit-operations.md)
   - Merge 2: `set_range`, row/column insertion/deletion, and sheet add/rename/delete
     on the same transaction/history spine.
   - Exit gate: each operation is advertised only after its cross-part golden tests
     pass.

6. [`2026-07-18-mcp-agent-interface.md`](2026-07-18-mcp-agent-interface.md)
   - Discoverable local stdio tools over the same core with structured errors and
     end-to-end agent-session tests.
   - Can start after plan 4; plan 5 capabilities appear automatically when ready.
   - Exit gate: an MCP client completes inspect → read → edit → history → revert
     without protocol pollution.

## Revision checkpoints

After every plan:

1. run its complete quality gate;
2. compare resulting public types and paths with all downstream plans;
3. update downstream plans instead of adding compatibility shims for unshipped APIs;
4. record architecture insights in the repository spec and the Dotall Obsidian
   brainstorming notes;
5. keep remote sharing, global cache, distributed locking, branching, and additional
   file formats outside XLSX v0.

## Completion definition

XLSX v0 is complete when:

- project-local `.all/` initializes and survives interruption;
- warm reads and repeated derivations are verified cache hits;
- agents discover natural selectors and semantic operations;
- source changes invalidate only regenerable artifacts;
- value and formula edits are transactional and fidelity-preserving;
- immutable history, diff, recovery, and revert work;
- supported broader operations pass impact and golden tests;
- CLI and MCP use the same core;
- `cargo fmt`, all tests, and Clippy pass with warnings denied.
