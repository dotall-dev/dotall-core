---
name: pptx
description: >-
  Work with .pptx files through Dotall MCP. Use when inspecting, reading, editing,
  versioning, or reverting PowerPoint decks in a Dotall workspace — prefer this over
  raw OOXML, ZIP, or XML manipulation.
---

# PPTX via Dotall MCP

Agent instructions for presentation work in Dotall workspaces. This skill teaches
**workflow and safety** — it is not a second API. Tool schemas and payloads come
from the Dotall MCP server at runtime.

## Prefer Dotall MCP

For `.pptx` files in an initialized Dotall workspace:

- **Do** use Dotall MCP tools (`dotall_capabilities`, `dotall_read`, `dotall_edit`, …).
- **Do not** unzip the deck, hand-edit slide XML, or rewrite DrawingML directly.
- **Do not** hardcode operation names or payload shapes — discover them per file.

Dotall performs **surgical OOXML patching**: only the target slide part changes;
untouched parts (media, theme, other slides) stay byte-for-byte identical.

## Prerequisites

1. Dotall MCP server running via stdio from the workspace root:

   ```bash
   cargo run -p dotall-mcp
   ```

2. Call `dotall_init` once per workspace (idempotent).

## Standard workflow

```text
dotall_capabilities or dotall_inspect
  → dotall_read
  → dotall_edit (stage)
  → dotall_staged (optional check)
  → dotall_apply OR session flush-on-close
  → dotall_history / dotall_diff / dotall_revert
```

### 1. Discover before every edit

Call **`dotall_capabilities`** or **`dotall_inspect`** before `dotall_read` or
`dotall_edit`.

From the response, capture:

- `format_id` — must be `pptx`.
- `selectors` — valid `dotall_read` kinds (`full`, `slide`, `notes`).
- `edit_capabilities[]` — supported operations, example payloads, and safety notes.
- `source_hash` — required for `dotall_edit` and `dotall_revert`.
- `suggested_reads` — usually one `slide` selector per slide.

v0 does **not** expose `dotall_deps` for presentations.

### 2. Read

- `selector_kind=full` — all slides as Markdown (`# Slide N` then shapes and table cells).
- `selector_kind=slide` — one slide. Identity can be `Slide 1`, `slide1`, or `1`.
  Table cells appear as `- **Table 1[0,1]**: text`.
- `selector_kind=notes` — speaker notes for that slide identity, if present.
- Inspect includes `table_count` per slide when tables are present.

Honor `max_tokens` and resume with the returned `continuation` cursor.

### 3. Stage edits

**`dotall_edit` stages only.** Supports a **single** operation per transaction.

Shape text:

```json
{
  "kind": "set_shape_text",
  "payload": { "slide": "Slide 1", "shape": "Title", "text": "World" }
}
```

Table cell text (`p:graphicFrame` → `a:tbl`):

```json
{
  "kind": "set_table_cell_text",
  "payload": {
    "slide": "Slide 1",
    "table": "Table 1",
    "row": 0,
    "col": 1,
    "text": "NEW"
  }
}
```

Add / delete slides:

```json
{ "kind": "add_slide", "payload": { "after": "Slide 1" } }
```

```json
{ "kind": "delete_slide", "payload": { "slide": "Slide 2" } }
```

Omit `after` on `add_slide` to append at the end. `delete_slide` rejects the sole remaining slide.

Safety:

- `set_shape_text`: text frames only (`p:sp` + `p:txBody`). Rejects SmartArt, charts, and grouped drawingML the writer cannot patch.
- `set_table_cell_text`: patches one cell; rejects out-of-range `row`/`col`. First `a:t` in the cell is replaced; later runs in that cell are cleared.
- `add_slide` / `delete_slide`: surgically update `presentation.xml`, `presentation.xml.rels`, and `[Content_Types].xml`; duplicate a blank slide template or remove the target slide part. Untouched slide/media parts stay byte-identical.
- Text ops patch only the target slide part; media and other slides stay byte-identical.
- No slide reorder yet.

### 4. Apply, history, revert

Same as XLSX: `dotall_apply`, `dotall_history`, `dotall_diff`, `dotall_revert`.
Snapshots are content-addressed ZIP parts (`pptx.snapshot-manifest`).
