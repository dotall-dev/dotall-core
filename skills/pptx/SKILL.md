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

Reorder slides (presentation.xml `sldIdLst` only):

```json
{ "kind": "move_slide", "payload": { "slide": "Slide 2", "to_index": 0 } }
```

Add a simple text box on a slide:

```json
{ "kind": "add_textbox", "payload": { "slide": "Slide 1", "name": "Callout", "text": "Agent note" } }
```

Delete a shape by slide + name:

```json
{ "kind": "delete_shape", "payload": { "slide": "Slide 1", "shape": "Callout" } }
```

Rename a shape (`cNvPr` name on the slide part):

```json
{ "kind": "rename_shape", "payload": { "slide": "Slide 1", "shape": "Title", "name": "Headline" } }
```

Set or clear bold on shape text runs (`a:rPr b`):

```json
{ "kind": "set_shape_bold", "payload": { "slide": "Slide 1", "shape": "Title", "bold": true } }
```

Set or clear italic on shape text runs (`a:rPr i`):

```json
{ "kind": "set_shape_italic", "payload": { "slide": "Slide 1", "shape": "Title", "italic": true } }
```

Set or clear underline on shape text runs (`a:rPr u`):

```json
{ "kind": "set_shape_underline", "payload": { "slide": "Slide 1", "shape": "Title", "underline": true } }
```

Set or clear font size on shape text runs (`a:rPr sz` from `size_pt`; `null` clears):

```json
{ "kind": "set_shape_font_size", "payload": { "slide": "Slide 1", "shape": "Title", "size_pt": 28.0 } }
```

Set or clear font name on shape text runs (`a:latin` / `a:ea` / `a:cs` typeface; `null` clears):

```json
{ "kind": "set_shape_font_name", "payload": { "slide": "Slide 1", "shape": "Title", "font": "Arial" } }
```

Set or clear solid sRGB text color on shape runs (`a:solidFill` / `a:srgbClr`; `null` clears):

```json
{ "kind": "set_shape_font_color", "payload": { "slide": "Slide 1", "shape": "Title", "color": "#FF0000" } }
```

Set or clear text highlight on shape runs (`a:highlight` / `a:srgbClr`; `null` clears):

```json
{ "kind": "set_shape_highlight", "payload": { "slide": "Slide 1", "shape": "Title", "color": "#FFFF00" } }
```

Set or clear strikethrough on shape runs (`a:rPr strike="sngStrike"` / `"noStrike"`):

```json
{ "kind": "set_shape_strikethrough", "payload": { "slide": "Slide 1", "shape": "Title", "strikethrough": true } }
```

Set or clear superscript/subscript on shape runs (`a:rPr baseline`; `null` clears):

```json
{ "kind": "set_shape_vert_align", "payload": { "slide": "Slide 1", "shape": "Title", "vert_align": "superscript" } }
```

Find/replace a substring inside one shape’s text (rejects empty `find` / no match; first-run rewrite like `set_shape_text`):

```json
{
  "kind": "replace_shape_text",
  "payload": {
    "slide": "Slide 1",
    "shape": "Title",
    "find": "Q3",
    "replace": "Q4"
  }
}
```

Speaker notes (existing notes slide part only):

```json
{
  "kind": "set_notes_text",
  "payload": { "slide": "Slide 1", "text": "Updated speaker notes" }
}
```

Omit `after` on `add_slide` to append at the end. `delete_slide` rejects the sole remaining slide.
`move_slide` uses a 0-based `to_index` (final position); rejects no-ops and single-slide decks.
Omit `name` on `add_textbox` to auto-name `TextBox N`; rejects duplicate names.
`delete_shape` removes a `p:sp` by name or element_id; rejects missing shapes (tables use graphicFrame and are not deleted here).
`rename_shape` updates `cNvPr` name; rejects empty/duplicate names (including table names on the same slide).
`set_shape_bold` upserts `a:rPr b` on each text run in the shape; rejects non-text shapes.
`set_shape_italic` upserts `a:rPr i` on each text run in the shape; rejects non-text shapes.
`set_shape_underline` upserts `a:rPr u="sng"` / `u="none"` on each text run in the shape; rejects non-text shapes.
`set_shape_font_size` upserts `a:rPr sz` (hundredths of a point from `size_pt`; null clears); rejects non-text shapes.
`set_shape_font_name` upserts `a:latin`/`a:ea`/`a:cs` typeface (null clears); rejects non-text shapes.
`set_shape_font_color` upserts `a:solidFill`/`a:srgbClr` from `#RRGGBB`/`RRGGBB` (null clears); rejects non-text shapes.
`set_shape_highlight` upserts `a:highlight`/`a:srgbClr` from `#RRGGBB`/`RRGGBB` (null clears); rejects non-text shapes.
`set_shape_strikethrough` upserts `a:rPr strike="sngStrike"` / `strike="noStrike"` on each text run; rejects non-text shapes.
`set_shape_vert_align` upserts `a:rPr baseline="30000"` (superscript) / `"-25000"` (subscript); null clears; rejects non-text shapes.
`replace_shape_text` replaces all occurrences of `find` in the shape text model, then patches the slide part; rejects empty `find` and no-match.

Safety:

- `set_shape_text`: text frames only (`p:sp` + `p:txBody`). Rejects SmartArt, charts, and grouped drawingML the writer cannot patch.
- `set_table_cell_text`: patches one cell; rejects out-of-range `row`/`col`. First `a:t` in the cell is replaced; later runs in that cell are cleared.
- `set_notes_text`: patches only the notes slide part; rejects slides without a notes part. First `a:t` is replaced; later runs cleared. Slide XML stays byte-identical.
- `add_slide` / `delete_slide`: surgically update `presentation.xml`, `presentation.xml.rels`, and `[Content_Types].xml`; duplicate a blank slide template or remove the target slide part. Untouched slide/media parts stay byte-identical.
- `move_slide`: reorders only `p:sldId` entries in `ppt/presentation.xml`. Slide parts, notes, rels, and Content_Types stay byte-identical.
- `set_shape_bold`: upserts `a:rPr b` on runs in the target shape’s `txBody`; other parts stay byte-identical.
- `set_shape_italic`: upserts `a:rPr i` on runs in the target shape’s `txBody`; other parts stay byte-identical.
- `set_shape_underline`: upserts `a:rPr u="sng"`/`none` on runs in the target shape’s `txBody`; other parts stay byte-identical.
- `set_shape_font_size`: upserts `a:rPr sz` on runs in the target shape’s `txBody`; other parts stay byte-identical.
- `set_shape_font_name`: upserts `a:latin`/`a:ea`/`a:cs` typeface on runs in the target shape’s `txBody`; other parts stay byte-identical.
- `set_shape_font_color`: upserts `a:solidFill`/`a:srgbClr` on runs in the target shape’s `txBody`; other parts stay byte-identical.
- `set_shape_highlight`: upserts `a:highlight`/`a:srgbClr` on runs in the target shape’s `txBody`; other parts stay byte-identical.
- `set_shape_strikethrough`: upserts `a:rPr strike` on runs in the target shape’s `txBody`; other parts stay byte-identical.
- `set_shape_vert_align`: upserts `a:rPr baseline` (superscript/subscript) on runs in the target shape’s `txBody`; other parts stay byte-identical.
- `replace_shape_text`: find/replace on shape text then first-run rewrite like `set_shape_text`; other parts stay byte-identical.
- `add_textbox`: inserts a `p:sp` text box (`txBox="1"`) into the target slide’s `spTree`; other parts stay byte-identical.
- `delete_shape`: removes the matching `p:sp` from the target slide part; other parts stay byte-identical.
- `rename_shape`: rewrites the matching `cNvPr` name attribute on the target slide part; other parts stay byte-identical.
- Text ops patch only the target slide part; media and other slides stay byte-identical.

### 4. Apply, history, revert

Same as XLSX: `dotall_apply`, `dotall_history`, `dotall_diff`, `dotall_revert`.
Snapshots are content-addressed ZIP parts (`pptx.snapshot-manifest`).
