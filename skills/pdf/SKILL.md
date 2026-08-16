---
name: pdf
description: >-
  Work with .pdf files through Dotall MCP. Use when inspecting pages, listing
  AcroForm fields, filling a text field, or reverting a PDF in a Dotall workspace.
  Do not rewrite page content streams.
---

# PDF via Dotall MCP

v0 is **form fill only**. Do not unzip, reprint, or rewrite page operators.

## Workflow

```text
dotall_capabilities or dotall_inspect
  → dotall_read (page / field / full)
  → dotall_edit set_form_field (stage)
  → dotall_apply OR flush-on-close
  → dotall_history / revert
```

### Read

- `selector_kind=page` — 1-based page number (`1`)
- `selector_kind=field` — field name (`Name`)
- `selector_kind=full` — pages plus field list

Text extraction is best-effort. Inspect `encrypted` and `has_signature` before editing.

### Edit

```json
{
  "kind": "set_form_field",
  "payload": { "name": "Name", "value": "Grace" }
}
```

Rejected: encrypted PDFs, signed/certified PDFs, read-only fields, checkboxes/radios in v0.

Snapshots are opaque whole-file blobs (not OOXML part manifests).
