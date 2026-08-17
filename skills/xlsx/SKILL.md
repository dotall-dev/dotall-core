---
name: xlsx
description: >-
  Work with .xlsx files through Dotall MCP. Use when inspecting, reading, editing,
  versioning, or reverting Excel workbooks in a Dotall workspace — prefer this over
  raw OOXML, ZIP, or XML manipulation.
---

# XLSX via Dotall MCP

Agent instructions for spreadsheet work in Dotall workspaces. This skill teaches
**workflow and safety** — it is not a second API. Tool schemas and payloads come
from the Dotall MCP server at runtime.

## Prefer Dotall MCP

For `.xlsx` files in an initialized Dotall workspace:

- **Do** use Dotall MCP tools (`dotall_capabilities`, `dotall_read`, `dotall_edit`, …).
- **Do not** unzip the workbook, hand-edit OOXML parts, or rewrite sheet XML directly.
- **Do not** hardcode operation names or payload shapes — discover them per file.

Dotall performs **surgical OOXML patching**: only target parts change; untouched
content (styles, charts, other sheets) stays byte-for-byte identical.

## Prerequisites

1. Dotall MCP server running via stdio from the workspace root:

   ```bash
   cargo run -p dotall-mcp
   ```

2. Call `dotall_init` once per workspace (idempotent).

3. For formula or structural edits, call `dotall_deps` on affected cells before
   writing when precedents/dependents matter.

## Standard workflow

```text
dotall_capabilities or dotall_inspect
  → dotall_read / dotall_deps
  → dotall_edit (stage)
  → dotall_staged (optional check)
  → dotall_apply OR session flush-on-close
  → dotall_history / dotall_diff / dotall_revert
```

### 1. Discover before every edit

Call **`dotall_capabilities`** (lightweight) or **`dotall_inspect`** (full summary)
before `dotall_read` or `dotall_edit` on a file.

From the response, capture:

- `format_id` — must be the XLSX handler (typically `xlsx`).
- `selectors` — valid `dotall_read` selector kinds (`full`, `range`, `sheet`,
  `ast_range`, `named_ranges`, `merges`, …).
- `edit_capabilities[]` — supported operation names, descriptions, **example**
  payloads, and safety notes.
- `source_hash` — required for `dotall_edit` and `dotall_revert` as
  `expected_source_hash`.
- `suggested_reads` — good starting selectors when exploring a workbook.
- Inspect `summary.sheets[].merges` — merged cell refs (e.g. `A1:B2`).
- Inspect `summary.named_ranges[]` — `{ name, formula }` (not names alone).
- Inspect `summary.style_table[]` — `{ style_id }` for non-default cell styles
  used in the workbook (ids only; no full style writer).
- Inspect `summary.sheets[].freeze_panes` — openpyxl-style freeze cell (e.g. `B2`),
  or absent/null when panes are not frozen.

Never assume an operation exists because another `.xlsx` supported it — always
re-check after external edits or a failed apply.

### 2. Read and understand

- **`dotall_read`**: token-budgeted projections. Use `selector_kind` + `selector`
  from capabilities (e.g. `range` + `Revenue!A1:D20`). Follow `continuation`
  cursors until the slice is complete.
  - `named_ranges` — JSON list of `{ name, formula, element_id }` (selector value
    unused).
  - `merges` — JSON `{ sheet, merges }` for one sheet (selector value = sheet name).
  - `ast_range` — JSON cells include optional `style_id` and `number_format`
    (e.g. `0%`, `$#,##0.00`) when the source cell has a non-default style.
- **`dotall_deps`**: precedents or dependents for a cell (`Sheet1!B2`). Use before
  formula changes, row/column inserts, or sheet renames/deletes.

### 3. Stage edits (never auto-applied)

**`dotall_edit` stages only** — it never writes the source file.

Required fields:

- `expected_source_hash` — from the latest `dotall_inspect` / `dotall_status`.
- `actor_id` — stable identifier for this agent/session.
- `operations` — array of `{ kind, payload }`.

Build each operation from **`edit_capabilities[].example`**:

```json
{
  "kind": "set_cell_value",
  "payload": { "sheet": "Sheet1", "address": "A1", "value": 42 }
}
```

Merge / unmerge (surgical worksheet `mergeCells` patch; overlapping merges are rejected):

```json
{ "kind": "merge_cells", "payload": { "sheet": "Sheet1", "range": "A1:B2" } }
```

```json
{ "kind": "unmerge_cells", "payload": { "sheet": "Sheet1", "range": "A1:B2" } }
```

Column width / row height (surgical worksheet `cols` / row attrs; other sheets byte-identical):

```json
{ "kind": "set_column_width", "payload": { "sheet": "Sheet1", "column": "A", "width": 18.5 } }
```

```json
{ "kind": "set_row_height", "payload": { "sheet": "Sheet1", "row": 1, "height": 30.0 } }
```

Freeze panes (surgical worksheet `sheetViews`; `null` or `A1` clears):

```json
{ "kind": "freeze_panes", "payload": { "sheet": "Sheet1", "cell": "B2" } }
```

Define or update a workbook-scoped named range (surgical `xl/workbook.xml` only;
leading `=` on `formula` is optional and stripped):

```json
{ "kind": "define_name", "payload": { "name": "Rate", "formula": "Inputs!$B$3" } }
```

Remove a workbook-scoped named range (surgical `xl/workbook.xml` only):

```json
{ "kind": "delete_name", "payload": { "name": "Rate" } }
```

Hide or unhide a worksheet (`state="hidden"` on the workbook sheet tag only):

```json
{ "kind": "hide_sheet", "payload": { "sheet": "Revenue", "hidden": true } }
```

Rejects hiding the last visible sheet. Worksheets stay byte-identical.

Set or clear a worksheet tab color (`sheetPr`/`tabColor` rgb AARRGGBB; `null` clears):

```json
{ "kind": "set_tab_color", "payload": { "sheet": "Inputs", "color": "FF4472C4" } }
```

Inspect surfaces `tab_color` per sheet when present. Other worksheets stay byte-identical.

Reuse `transaction_id` when retrying the same staged edit after a transient error.

Check pending work with **`dotall_staged`**.

### 4. Apply or flush

- **`dotall_apply`**: commit one transaction (`transaction_id`) or all staged edits
  (`all: true`). Creates immutable history versions.
- **Session flush-on-close (default)**: when the MCP session ends gracefully, Dotall
  best-effort applies all staged transactions across tracked files. Disable with
  `--no-flush-on-close` or `DOTALL_MCP_FLUSH_ON_CLOSE=0`.

Prefer explicit **`dotall_apply`** when you need the new `source_hash` or history
entry before continuing. Rely on flush only when ending a session with acceptable
staged work.

### 5. History and revert

- **`dotall_history`**: committed versions with actors and semantic summaries.
- **`dotall_diff`**: ordered changes for one version.
- **`dotall_revert`**: stages restoration of a version's pre-edit snapshot. Still
  requires **`dotall_apply`** (or flush-on-close) — revert does not auto-apply.

Use **`dotall_discard`** to drop a staged transaction without touching source bytes.

## Safety rules

| Rule | Why |
|------|-----|
| **Stage-default** | `dotall_edit` never mutates the `.xlsx`. Source changes only after `dotall_apply` or flush-on-close. |
| **`expected_source_hash`** | Prevents stale writes when the file changed on disk since your last inspect. On mismatch, re-inspect and retry. |
| **Capability-driven ops** | Unsupported `kind` values return structured errors listing available operations. |
| **Structural rejects** | Row/column/sheet ops validate impact first (formula rewrites, chart/table/name references). Rejection is intentional — do not bypass with raw file edits. |
| **No formula evaluation** | Dotall stores and patches formulas; it does not compute results. |
| **Charts/pivots preserved, not edited** | Do not attempt chart or pivot mutation; preserve them by using Dotall edits only. |

When a structural edit is rejected, read the error, inspect dependents with
`dotall_deps`, adjust the plan, or choose a narrower cell-level edit.

## Recovery patterns

- **Stale hash after external edit**: `dotall_inspect` → new hash → re-stage.
- **Unsure what staged**: `dotall_staged` → `dotall_apply` or `dotall_discard`.
- **Wrong committed version**: `dotall_history` → `dotall_diff` → `dotall_revert` →
  `dotall_apply`.
- **Abandoned session with flush disabled**: staged edits remain in `.all/`; resume
  with `dotall_staged` and apply or discard explicitly.

## What this skill is not

- Not a duplicate tool catalog — use MCP `tools/list` and tool descriptions.
- Not a promise of every Excel feature — only advertised `edit_capabilities` are
  supported.
- Not a substitute for reading the file — always inspect/read before editing.
