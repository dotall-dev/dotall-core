# XLSX Write + History Design

**Date:** 2026-07-20  
**Status:** Merge 1 implemented (polish complete); Merge 2 pending  
**Related:**  
- `docs/superpowers/specs/2026-07-19-xlsx-translation-ast-design.md`  
- `docs/superpowers/plans/2026-07-18-xlsx-formula-dependencies.md`  
- `docs/superpowers/plans/2026-07-18-transactional-xlsx-edits.md`  
- `docs/superpowers/plans/2026-07-18-broaden-xlsx-edit-operations.md`  
- `docs/specs/xlsx-engine-v0.md`

## Goal

Let agents stage and apply XLSX edits without breaking spreadsheets: surgical OOXML patching, optimistic concurrency, idempotent transactions, forensic edit history, semantic diffs, crash recovery, and revert — with formula-dependency awareness before the first write lands.

## Decisions

| Topic | Choice |
|-------|--------|
| End-state op coverage | **Structural goal (C):** cell + range + row/col + sheet ops over time |
| Delivery sequence | **Two merges (B):** Merge 1 = cell value/formula + full transaction/history spine; Merge 2 = structural ops on that spine |
| Formula deps | **Required before Merge 1 (A):** build `xlsx.formula-dependencies` first |
| History depth | **Forensic spine (C)** with semantic agent-facing `history` / `diff` projections |
| Default apply policy | **Stage-only (B):** source unchanged until explicit `apply`, or MCP session-close flush |
| Writer strategy | **Layered transaction spine + surgical ZIP/XML patch** — not full reserialize |
| Charts / pivots / VBA | **Preserve-only** (no mutation) |

## 1. Delivery sequence

| Phase | Ships | Does not ship yet |
|-------|--------|-------------------|
| **0 — Formula deps** | Derived graph + forward/reverse queries | Any source mutation |
| **Merge 1 — Cell writes + spine** | `set_cell_value`, `set_cell_formula`; stage/apply; forensic history; revert; surgical cell patch | Row/col/sheet structural ops |
| **Merge 2 — Structural** | `set_range`, insert/delete row/col, add/rename/delete sheet on the same spine | Chart/pivot/VBA mutation |

Existing plans should be revised to match this design (stage-default, MCP close-flush, forensic history fields, deps-before-writes). Prefer this doc when they conflict.

## 2. Stage / apply lifecycle

Default agent workflow **stages** edits. The workbook on disk does not change until apply.

```text
edit(ops, expected_source_hash, tx_id?)
  → detect / ensure fresh model (+ formula deps)
  → format validate (semantic ops + dependency impact)
  → durable stage under state/edits/staging/ + journal intent
  → return staged preview (semantic diff, dependents, tx_id)
  → source file UNCHANGED

apply(tx_id | --all)   OR   MCP session close (flush staged)
  → re-check expected source hash
  → snapshot pre-apply source bytes
  → surgical patch → atomic replace source (same filesystem)
  → refresh model / derived / views
  → append forensic history version
  → clear or advance staging
  → return version, hashes, revert handle
```

### Rules

- Failed validate or apply leaves the source untouched.
- Staging is retained or marked failed for recovery classification.
- `tx_id` makes retries idempotent: duplicate edit/apply returns the original staged or applied result.
- `discard` abandons staged work; prefer a journal/cancel audit entry without creating an applied history version.
- MCP session-close apply is a **flush policy** over the same apply path — not a second writer implementation.
- One local writer lock per tracked object during apply.

## 3. History records and projections

### On-disk layout

Under `.all/objects/<relative-path>/state/edits/`:

```text
staging/
  <tx_id>.json
transactions/
  <tx_id>.journal.jsonl
history/
  v001.json
  …
  snapshots/
    manifests/
      <package_blake3>.json
    parts/
      <part_blake3>
```

Snapshot storage is defined by the superseding
`2026-08-12-snapshot-delta-storage-design.md`: XLSX packages are reconstructed
byte-for-byte from content-addressed ZIP slices, while unchanged parts are shared
across history versions.

### Committed version record (forensic)

```json
{
  "version": 1,
  "tx_id": "…",
  "status": "applied",
  "timestamp": "…",
  "actor": { "kind": "cli|mcp|system", "id": "optional" },
  "ops": [],
  "semantic_diff": [
    {
      "target": "Revenue!B12",
      "element_id": "c_…",
      "change": "formula",
      "before": "=A1",
      "after": "=A1*1.1"
    }
  ],
  "dependency_impact": {
    "forward": ["Revenue!C12", "Summary!B2"],
    "notes": ["refs parsed; values not evaluated"]
  },
  "before_source_hash": "…",
  "after_source_hash": "…",
  "snapshot_ref": "<blake3>",
  "revert_of": null
}
```

### Agent-facing projections

| Command | Returns |
|---------|---------|
| `history` | Compact list: version, time, actor, one-line summary, op count |
| `diff <v>` / `diff <a> <b>` | Semantic diff + dependency impact |
| `revert <v>` | Stages a revert transaction that restores the snapshot; apply (or MCP close flush) still required; creates a **new** history version with `revert_of` |

### History rules

- History is append-only; revert never deletes past versions.
- Cancel/discard: journal/cancel audit only — no applied history version.
- Pre-apply snapshots are required for revert; snapshot GC is an explicit later operation (non-goal for Merge 1).

## 4. Merge 1 operations

```text
set_cell_value   { sheet | element_id, address?, value }
set_cell_formula { sheet | element_id, address?, formula }
```

- Targets use hybrid ids and/or `Sheet!Address` (same dual surface as reads).
- Validate against the current canonical model and formula-dependency graph.
- Unknown sheet/cell → structured error with alternatives.
- Formula strings remain opaque; no evaluation engine in v0.
- Dependency impact lists reference dependents for preview and history.

## 5. Surgical writer

- Input: original `.xlsx` ZIP + validated ops.
- Patch only required parts (typically worksheet XML, shared strings, and calc-related parts when needed).
- Every untouched ZIP entry remains **byte-identical** (guarded by golden round-trip tests).
- Do not apply via full high-level re-serialization libraries.
- After apply: refresh model, rebuild/invalidate derived deps, invalidate views, update fingerprints/manifest.

## 6. Concurrency and recovery

| Guard | Behavior |
|-------|----------|
| `expected_source_hash` | Required on `edit`; re-checked on `apply`; mismatch rejects with no write |
| Local lock | One apply at a time per tracked object |
| Idempotent `tx_id` | Duplicate requests return the original result |
| Crash recovery | Journal classifies incomplete transactions; recover leaves source unchanged or completes only if commit was already durable |

## 7. CLI / MCP surface (Merge 1)

```text
edit <file> …                 # stage
staged / status --staged      # list pending
apply <file> [--tx …] | apply --all
discard <tx>
history <file>
diff <file> <version>
revert <file> <version>       # stages revert; apply still required unless MCP close flush
```

MCP tools mirror the same verbs. Session close runs apply-flush for remaining staged transactions according to policy (configurable later; default flush-on-close per product decision).

## 8. Merge 2 preview (same spine)

Structural operations (`set_range`, insert/delete row/col, add/rename/delete sheet) reuse stage/apply/history/revert unchanged.

Each structural op is advertised only after:

- complete impact analysis against the formula-dependency graph (or safe preflight rejection);
- golden tests proving untouched OOXML parts stay byte-identical where required.

Charts, pivots, and VBA remain preserve-only.

## 9. Relationship to formula dependencies (Phase 0)

Phase 0 must deliver:

- formula reference parsing (no evaluation);
- cached derived artifact `xlsx.formula-dependencies`;
- forward/reverse queries for agents;
- invalidation on source/model change.

Merge 1 consumes that graph in validate, staged preview, and history `dependency_impact`.

## Non-goals (this design)

- Formula evaluation / calc engine
- Full xlsx reserialization as the apply path
- Chart / pivot / VBA editing
- Branching history
- Multi-agent distributed locking
- Automatic snapshot garbage collection in Merge 1
- Auto-apply as the default agent path (stage-only is default)

## Plan revision notes

When turning this into implementation plans:

1. Execute / revise formula-deps plan as Phase 0.  
2. Revise transactional edits plan for stage-default, forensic history fields, MCP close-flush hook, and deps prerequisite.  
3. Keep broaden-ops plan as Merge 2 on the same spine — do not invent a second history system.
