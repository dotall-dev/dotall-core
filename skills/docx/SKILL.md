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

Dotall patches only `word/document.xml` for v0 paragraph edits. Untouched ZIP
parts stay byte-identical.

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

### Read

- `selector_kind=full` — numbered body paragraphs (`0. text`).
- `selector_kind=paragraphs` — `0:2` (start inclusive, end exclusive) or a single index `0`.

v0 **skips table paragraphs**. Inspect `skipped_tables` if the document has tables.

v0 does not expose `dotall_deps`.

### Edit

Single operation per transaction:

```json
{
  "kind": "set_paragraph_text",
  "payload": { "index": 1, "text": "Gamma" }
}
```

`element_id` is also accepted instead of `index`.

Rejected: tracked changes (`w:ins`/`w:del`), content controls (`w:sdt`), fields.
No header/footer/comment/style/table edits in v0.

Keeps `w:pPr` and clones the first run’s `w:rPr` when present.
