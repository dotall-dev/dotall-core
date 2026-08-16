---
name: pdf
description: >-
  Work with .pdf files through Dotall MCP. Use when inspecting pages, listing
  AcroForm fields, filling a text or checkbox field, or reverting a PDF in a
  Dotall workspace. Do not rewrite page content streams.
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
Inspect `metadata` for `/Info` Title / Author / Subject / Creator / Producer when present.
Checkbox fields (`btn`) may include `export_values` (e.g. `Yes`, `Off`) from `/AP /N`.
Choice fields (`ch`) expose `options` from `/Opt` on inspect and field read.

### Edit

Text / choice:

```json
{
  "kind": "set_form_field",
  "payload": { "name": "Name", "value": "Grace" }
}
```

Choice (`Ch`): set `value` to one of the field `options` (export string from `/Opt`).

Checkbox (`Btn`): use `On` / `Off`, or an explicit export value from `export_values`:

```json
{
  "kind": "set_form_field",
  "payload": { "name": "Agree", "value": "On" }
}
```

`On` maps to the sole non-`Off` export value when unambiguous; radios with multiple on-states require an explicit export value.

Rejected: encrypted PDFs, signed/certified PDFs, read-only fields, ambiguous radios without an export value.

Snapshots are opaque whole-file blobs (not OOXML part manifests).
