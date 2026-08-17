---
name: docx
description: >-
  Work with .docx files through Dotall MCP. Use when inspecting, reading, editing,
  versioning, or reverting Word documents in a Dotall workspace — prefer this over
  raw OOXML, ZIP, or XML manipulation.
---

# DOCX via Dotall MCP

Agent instructions for Word work in Dotall workspaces. This skill teaches
**workflow and safety** — it is not a second API. Discover payloads from the
Dotall MCP server at runtime.

## Prefer Dotall MCP

For `.docx` files in an initialized Dotall workspace:

- **Do** use Dotall MCP tools (`dotall_capabilities`, `dotall_read`, `dotall_edit`, …).
- **Do not** unzip the package or rewrite `word/document.xml` by hand.
- **Do not** hardcode operations — discover them per file.

Dotall patches only the target story part for paragraph edits (`word/document.xml`,
`word/header*.xml`, or `word/footer*.xml`). Hyperlink edits also patch
`word/_rels/document.xml.rels`. Untouched ZIP parts stay byte-identical.

## Prerequisites

```bash
cargo run -p dotall-mcp
```

Call `dotall_init` once per workspace.

## Workflow

```text
dotall_capabilities or dotall_inspect
  → dotall_read
  → dotall_edit (stage)
  → dotall_apply OR session flush-on-close
  → dotall_history / dotall_diff / dotall_revert
```

## Pack / needle-finding

Pack / needle-finding: call `dotall_search` (query like `10%` or `Rate`) instead
of unzipping OOXML. Then `read`/`edit` using returned selectors. `dotall viz` is
for humans inspecting `.all/`, not required in the edit loop.

### Read

- `selector_kind=full` — body paragraphs, plus Headers/Footers sections when present.
- `selector_kind=paragraphs` — `0:2` (start inclusive, end exclusive) or a single index `0`.
- `selector_kind=headers` — flat list `0:1`, or part-qualified `header1:0`.
- `selector_kind=footers` — flat list `0:1`, or part-qualified `footer1:0`.

**Body indexing:** paragraphs are numbered in **document order** across the body of
`word/document.xml`, including table cell paragraphs. A body paragraph followed
by a 1×2 table yields indices `0` (body), `1` (first cell), `2` (second cell).
Inspect reports `table_count` and `skipped_tables: false` when tables are modeled.

**Header/footer indexing:** separate from body. Inspect lists `headers` /
`footers` as `{ part, index, text }` where `part` is the stem (`header1`,
`footer1`) and `index` is **within that part**. Flat read selectors use sorted
part order, then within-part index.

v0 does not expose `dotall_deps`.

### Edit

Single operation per transaction:

```json
{
  "kind": "set_paragraph_text",
  "payload": { "index": 1, "text": "Gamma" }
}
```

`element_id` is also accepted instead of `index`. Same op targets body or
table-cell paragraphs by document-order index.

Insert after an existing paragraph (body or table cell). Subsequent indices shift:

```json
{
  "kind": "insert_paragraph",
  "payload": { "after": 2, "text": "Action: confirm owners before Friday." }
}
```

Delete by document-order `index` or `element_id` (body or table cell). Subsequent
indices shift down:

```json
{
  "kind": "delete_paragraph",
  "payload": { "index": 3 }
}
```

Set paragraph style (`w:pStyle` / `style_id`) without rewriting `styles.xml`:

```json
{
  "kind": "set_paragraph_style",
  "payload": { "index": 2, "style_id": "Heading1" }
}
```

`element_id` is also accepted instead of `index`. Upserts `w:pStyle` inside
`w:pPr`; other paragraph properties and runs stay intact. The style must already
exist in the document’s `styles.xml` for Word to resolve it — Dotall only sets
the reference.

Set paragraph alignment (`w:jc`: `left`, `center`, `right`, `both` / `justify`):

```json
{
  "kind": "set_paragraph_alignment",
  "payload": { "index": 1, "alignment": "center" }
}
```

Set or clear bold on all runs in a body/table paragraph (`w:b`):

```json
{
  "kind": "set_paragraph_bold",
  "payload": { "index": 1, "bold": true }
}
```

Set or clear italic on all runs in a body/table paragraph (`w:i`):

```json
{
  "kind": "set_paragraph_italic",
  "payload": { "index": 1, "italic": true }
}
```

Set or clear underline on all runs in a body/table paragraph (`w:u`):

```json
{
  "kind": "set_paragraph_underline",
  "payload": { "index": 1, "underline": true }
}
```

Set or clear font size on all runs in a body/table paragraph (`w:sz` / `w:szCs` from `size_pt`; `null` clears):

```json
{
  "kind": "set_paragraph_font_size",
  "payload": { "index": 1, "size_pt": 14.0 }
}
```

Set or clear font name on all runs in a body/table paragraph (`w:rFonts`; `null` clears):

```json
{
  "kind": "set_paragraph_font_name",
  "payload": { "index": 1, "font": "Arial" }
}
```

Set or clear font color on all runs in a body/table paragraph (`w:color`; `null` clears):

```json
{
  "kind": "set_paragraph_font_color",
  "payload": { "index": 1, "color": "#C00000" }
}
```

Set or clear highlight on all runs in a body/table paragraph (`w:highlight`; Word names like `yellow`; `null` clears):

```json
{
  "kind": "set_paragraph_highlight",
  "payload": { "index": 1, "color": "yellow" }
}
```

Set or clear strikethrough on all runs in a body/table paragraph (`w:strike`):

```json
{
  "kind": "set_paragraph_strikethrough",
  "payload": { "index": 1, "strikethrough": true }
}
```

Set or clear superscript/subscript on all runs in a body/table paragraph (`w:vertAlign`; `null` clears):

```json
{
  "kind": "set_paragraph_vert_align",
  "payload": { "index": 1, "vert_align": "superscript" }
}
```

Set or clear small-caps/all-caps on all runs in a body/table paragraph (`w:smallCaps` / `w:caps`; `null` clears):

```json
{
  "kind": "set_paragraph_caps",
  "payload": { "index": 1, "caps": "small" }
}
```

Set or clear an external hyperlink wrapping a body/table paragraph’s runs (`w:hyperlink` + `word/_rels/document.xml.rels`; `http://` / `https://` / `mailto:`; `null` clears; empty string rejected):

```json
{
  "kind": "set_paragraph_hyperlink",
  "payload": { "index": 0, "url": "https://example.com" }
}
```

`element_id` is also accepted instead of `index`. Setting wraps existing runs in one `w:hyperlink r:id` (or updates the existing wrapper and Target). Clearing unwraps inner `w:r` and removes the matching hyperlink Relationship. Patches `word/document.xml` and `word/_rels/document.xml.rels` only.

Find/replace a substring inside one body/table paragraph (rejects empty `find` / no match; first-run rewrite like `set_paragraph_text`):

```json
{
  "kind": "replace_paragraph_text",
  "payload": { "index": 1, "find": "draft", "replace": "final" }
}
```

Header / footer:

```json
{
  "kind": "set_header_paragraph_text",
  "payload": { "part": "header1", "index": 0, "text": "CONFIDENTIAL" }
}
```

```json
{
  "kind": "set_footer_paragraph_text",
  "payload": { "part": "footer1", "index": 0, "text": "Page 1" }
}
```

Bare `index` (without `part`) selects from the flat header/footer list in sorted
part order. Prefer `part` + `index` for clarity.

Rejected: tracked changes (`w:ins`/`w:del`), content controls (`w:sdt`), fields.

**Multi-run fidelity:** keeps `w:pPr` untouched. Plain `text` clones the **first
text-run’s** `w:rPr` (not paragraph-mark rPr inside `w:pPr`) and clears subsequent
runs. Optional `runs` preserves per-run formatting:

```json
{
  "kind": "set_paragraph_text",
  "payload": {
    "index": 1,
    "runs": [
      { "text": "Pilot complete; " },
      { "text": "expanding to PPTX and PDF." }
    ]
  }
}
```

When both `text` and `runs` are provided, `text` must equal the concatenation of
run texts. Header/footer ops accept the same `runs` shape.
