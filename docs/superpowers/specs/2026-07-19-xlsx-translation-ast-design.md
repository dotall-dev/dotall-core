# XLSX Translation AST Design

**Date:** 2026-07-19  
**Status:** Approved for planning  
**Related:** `docs/specs/core-format-architecture.md`, `docs/specs/xlsx-engine-v0.md`, `docs/superpowers/plans/2026-07-18-xlsx-read-pipeline.md`

## Goal

Make Dotall’s XLSX **typed JSON AST and projections** a sturdy translation layer between Excel and agents — not a parser dump. Agents navigate structured maps and readable slices; edits and dependencies hang off a versioned canonical model.

## Decisions

| Topic | Choice |
|-------|--------|
| Agent surface | **Dual dialect:** Markdown default for reading; structured JSON for inspect maps and precise `ast_range` slices. Canonical `model.json` is not the normal agent UX. |
| Element IDs | **Hybrid:** opaque stable `element_id` + always-present human selector (`Sheet!Address`) in projections. |
| Intelligence | **Split:** cell-faithful canonical model; detections live in derived artifacts. |
| Model fidelity now | **B:** sheets, sparse cells, values, opaque formulas, named ranges, merges, style/number-format refs. |
| Long-term fidelity | **Ladder to C** (Excel-class coverage) via schema versions — not a claim that v1 is Excel. |
| Excel parity | **Feature-class hybrid:** model+edit values/formulas/structure/formatting over time; charts/pivots/VBA/etc. remain **preserve-only** until there is a strong agent need. |
| Architecture | **Layered artifacts** (model / derived / views), not a universal IR graph from day one. |

## 1. Artifact layers

Every tracked workbook under `.all/objects/<relative-path>/cache/`:

| Layer | Path | Schema family | Role |
|-------|------|---------------|------|
| Canonical model | `model/model.json` | `xlsx.workbook` | Cell-faithful truth. Edit target. No guesses. |
| Derived | `derived/<processor>.json` | `xlsx.structure`, later `xlsx.formula-dependencies`, … | Regenerable intelligence. |
| Views | `views/<request_hash>.json` | rendered read/inspect responses | Budgeted agent dialects. |

### Shared envelope

Core-owned wrapper; format-owned payload:

```json
{
  "format_id": "xlsx",
  "schema_id": "xlsx.workbook",
  "schema_version": 1,
  "payload": {}
}
```

Cache binding always includes at least `source_hash` and producer id/version so a changed source or schema never silently reuses stale JSON.

### Hard rules

- Core stores and routes envelopes; only `dotall-xlsx` interprets payloads.
- Unmodeled Excel features are **omitted** from the model, not faked. Surgical OOXML preserves them on write.
- Derived never writes back into the canonical model.
- Agents default to `inspect` / `read` views; they do not browse raw `model.json` as normal UX.

## 2. Canonical model `xlsx.workbook` v1

### Top level

```json
{
  "workbook_id": "wb_…",
  "sheets": [],
  "named_ranges": [],
  "style_table": [],
  "unmodeled": {
    "charts": "preserved",
    "pivots": "preserved",
    "vba": "preserved",
    "other_ooxml_parts": "preserved"
  }
}
```

`unmodeled` is an explicit capability map — not a dump of those parts.

### Sheet

```json
{
  "element_id": "sh_…",
  "name": "Revenue",
  "index": 0,
  "dimensions": { "rows": 120, "cols": 12 },
  "merges": ["A1:C1", "E2:E4"],
  "cells": []
}
```

Cells are **sparse** (empty unmarked cells omitted). `dimensions` is the used range.

### Cell

```json
{
  "element_id": "c_…",
  "address": "B12",
  "row": 12,
  "col": 2,
  "value": { "kind": "float", "value": 42.5 },
  "formula": "=A12*1.1",
  "style_id": "st_3",
  "number_format": "0.00%"
}
```

- `value.kind`: `empty | string | float | integer | boolean | error | datetime`
- `datetime` in v1: ISO string (Excel serial may be added in a later schema version if needed)
- `formula`: opaque Excel formula string when present (reference graph is derived elsewhere)
- `style_id` / `number_format`: modeled for fidelity ladder B; `style_table` may start minimal (font/fill/border/align stubs)

### Named range

```json
{
  "element_id": "nr_…",
  "name": "TaxRate",
  "formula": "Assumptions!$B$2"
}
```

### Hybrid IDs

| Node | `element_id` | Human locator |
|------|--------------|---------------|
| Workbook | `wb_<stable>` | path / source hash in cache meta |
| Sheet | `sh_<stable>` | `name` (+ `index`) |
| Cell | `c_<stable>` | `Sheet!Address` (e.g. `Revenue!B12`) |
| Style | `st_<n>` or `st_<hash>` | optional label in projections |

**Stability rules (v1):**

- IDs are opaque at the API boundary.
- Reparse of an unchanged source (same content hash) must reproduce the same IDs (deterministic generator from content + schema version).
- Dotall edits update the model in place and **preserve** `element_id`s.
- External reshaping of the file (user edits in Excel that change layout) marks the object stale; rebuild may remint IDs.
- Projections always include both `element_id` and the human selector.

### Out of v1 model (preserve-only)

Charts, pivots, VBA, drawings, full conditional-formatting trees, rich-text run trees (flatten to string + flag if needed).

## 3. Derived `xlsx.structure` v1

Regenerable intelligence. Cache key includes at least:

```text
(source_hash, model_schema_id, model_schema_version, processor_id, processor_version)
```

Sketch:

```json
{
  "sheets": [
    {
      "sheet_element_id": "sh_…",
      "sheet_name": "Revenue",
      "header_row": 1,
      "tables": [
        {
          "element_id": "tbl_…",
          "selector": "Revenue!A1:D40",
          "confidence": 0.82,
          "columns": [
            { "name": "Month", "selector": "Revenue!A:A" },
            { "name": "Amount", "selector": "Revenue!B:B" }
          ]
        }
      ],
      "key_cells": [
        {
          "selector": "Revenue!B12",
          "element_id": "c_…",
          "reason": "formula_sink"
        }
      ]
    }
  ],
  "warnings": ["header detection low confidence on 'Notes'"]
}
```

**Rules:** guesses carry `confidence` and/or `warnings`. Agents may follow these selectors; edits still target canonical cells/sheets by `element_id` or `Sheet!Address`. Formula dependencies use a separate derived schema later (`xlsx.formula-dependencies`).

## 4. Agent projection dialects

| Dialect | When | Shape |
|---------|------|--------|
| Inspect map | `inspect` | Structured JSON: sheets, dims, capabilities, suggested reads, preserved flags, optional structure highlights |
| Markdown | default `read` for sheet/range/preview | Tables + formula annotations + drill-down hints |
| Structured slice | `read` with `ast_range` (or explicit JSON format) | AST subset: cells with hybrid IDs + values/formulas |

### Inspect sketch

```json
{
  "format_id": "xlsx",
  "source_hash": "…",
  "summary": {
    "sheets": [
      { "name": "Revenue", "rows": 120, "cols": 12, "formula_count": 18 }
    ],
    "named_ranges": ["TaxRate"],
    "preserved": ["charts", "pivots", "vba"]
  },
  "capabilities": [
    "inspect",
    "read.sheet",
    "read.range",
    "read.full",
    "read.ast_range"
  ],
  "suggested_reads": [
    {
      "description": "Revenue table preview",
      "selector": { "kind": "range", "value": "Revenue!A1:D20" }
    },
    {
      "description": "Exact cells as JSON",
      "selector": { "kind": "ast_range", "value": "Revenue!A1:D20" }
    }
  ]
}
```

### Read defaults

- No selector → summary + light Markdown preview
- `sheet` / `range` → Markdown
- `ast_range` → structured JSON slice
- Token budgets and continuations apply to all dialects
- Provenance: `state/access/log.jsonl` records what was served

## 5. Versioning and invalidation

### Schema evolution (B → C)

- Every payload carries `(format_id, schema_id, schema_version)`.
- Additive optional fields within a schema version line; unknown fields ignored by older readers where safe.
- Breaking changes bump `schema_version`; incompatible cache entries miss and rebuild from source. Durable history/state is not wiped.
- Cache producer identity includes schema id/version and format/processor version.

### Invalidation matrix

| Event | Model | Derived | Views | History |
|-------|--------|---------|-------|---------|
| Source mtime/size/hash change | miss → reparse | miss | miss | keep |
| Schema/processor version bump | miss if incompatible | miss | miss | keep |
| Dotall edit apply | update in place | miss affected | miss | append |
| Manual cache wipe | rebuild on demand | rebuild | rebuild | keep |

Warm path must verify **live** source freshness before trusting cached model. Manifest fingerprint alone is not sufficient.

## 6. Engine read contract

```text
inspect/read(file, selector?, budget?)
  → detect format
  → auto-register if needed
  → verify source freshness
  → load or parse canonical xlsx.workbook
  → optionally ensure derived xlsx.structure (for inspect hints)
  → render dialect (inspect JSON | Markdown | ast_range JSON)
  → apply token budget + continuation
  → write view cache keyed by request hash
  → append access provenance
  → return content + next_actions / suggested selectors
```

Additional rules:

- Format handler returns envelopes; core does not interpret cell JSON.
- Projections are pure functions of `(model [, derived], request)` plus budget.
- `ast_range` is a slice of the canonical model, not a second source of truth.
- Missing capability → structured error with available alternatives and an example selector.

## 7. Scope relative to existing plans

| In translation-layer MVP (revise read pipeline) | Later |
|--------------------------------------------------|--------|
| Envelope + `xlsx.workbook` v1 (B fields) | Formula-dependency derived graph |
| Hybrid IDs + dual dialects | Broader edit / formatting ops |
| Inspect map + Markdown + `ast_range` | MCP over the same contracts |
| Lightweight `xlsx.structure` (or stub with clear follow-ups) | Richer structure heuristics |
| Cache, freshness, budget, provenance | Surgical OOXML edit apply |

The read-pipeline plan must be updated so its `WorkbookModel` and projections match this design instead of a minimal cell dump.

## Non-goals (this design)

- Formula evaluation
- Universal cross-format IR graph
- Modeling charts/pivots/VBA for mutation
- Agents consuming raw `cache/model/model.json` as the default interface
- Remote artifact sharing
