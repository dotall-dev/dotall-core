---
name: pdf
description: >-
  Work with .pdf files through Dotall MCP. Use when inspecting pages, listing
  AcroForm fields, filling or clearing a text/checkbox/choice/radio field,
  setting /Info metadata, or reverting a PDF in a Dotall workspace. Do not
  rewrite page content streams.
---

# PDF via Dotall MCP

v0 is **form fill + document metadata**. Do not unzip, reprint, or rewrite page
operators.

## Workflow

```text
dotall_capabilities or dotall_inspect
  → dotall_read (page / field / full)
  → dotall_edit set_form_field | set_form_fields | set_form_field_readonly | set_form_field_required | set_form_field_multiline | set_form_field_password | set_form_field_max_length | clear_form_field | clear_all_form_fields | set_document_metadata (stage)
  → dotall_apply OR flush-on-close
  → dotall_history / revert
```

### Read

- `selector_kind=page` — 1-based page number (`1`)
- `selector_kind=field` — field name (`Name`)
- `selector_kind=full` — pages plus field list

Text extraction is best-effort but operator-aware (`Tj` / `TJ` / `'` / `"`, line
breaks from `Td` / `T*`). Inspect `encrypted` and `has_signature` before editing.
Inspect `metadata` for `/Info` Title / Author / Subject / Creator / Producer when present.
Checkbox and radio fields (`btn`) may include `export_values` (e.g. `Yes`, `Off`,
or radio states `Low` / `Medium` / `High`) from `/AP /N` across widgets.
Choice fields (`ch`) expose `options` from `/Opt` on inspect and field read.
Inspect surfaces `read_only` per field from `/Ff` bit 1.

### Edit

Text / choice:

```json
{
  "kind": "set_form_field",
  "payload": { "name": "Name", "value": "Grace" }
}
```

Choice (`Ch`): set `value` to one of the field `options` (export string from `/Opt`).

Checkbox (`Btn`, single on-state): use `On` / `Off`, or an explicit export value from
`export_values`:

```json
{
  "kind": "set_form_field",
  "payload": { "name": "Agree", "value": "On" }
}
```

`On` maps to the sole non-`Off` export value when unambiguous.

Radio group (`Btn` with multiple on-states): always pass an explicit export value
(e.g. `High`). Do not use bare `On` — validation rejects it as ambiguous.

```json
{
  "kind": "set_form_field",
  "payload": { "name": "Priority", "value": "High" }
}
```

Clear a field (blank text/choice, `Off` for checkbox/radio):

```json
{
  "kind": "clear_form_field",
  "payload": { "name": "Name" }
}
```

Clear every non-read-only AcroForm field in one transaction:

```json
{
  "kind": "clear_all_form_fields",
  "payload": {}
}
```

Set multiple fields in one transaction (name→value map; same rules as `set_form_field`):

```json
{
  "kind": "set_form_fields",
  "payload": {
    "fields": {
      "Name": "Ada Lovelace",
      "Agree": "On"
    }
  }
}
```

Lock or unlock a field (`/Ff` ReadOnly bit):

```json
{
  "kind": "set_form_field_readonly",
  "payload": { "name": "Name", "readonly": true }
}
```

Mark a field required or optional (`/Ff` Required bit):

```json
{
  "kind": "set_form_field_required",
  "payload": { "name": "Name", "required": true }
}
```

Mark a text field multiline or single-line (`/Ff` Multiline bit; `tx` only):

```json
{
  "kind": "set_form_field_multiline",
  "payload": { "name": "Name", "multiline": true }
}
```

Mark a text field as password or clear (`/Ff` Password bit; `tx` only):

```json
{
  "kind": "set_form_field_password",
  "payload": { "name": "Name", "password": true }
}
```

Set or clear a text field character limit (`/MaxLen`; `tx` only; `null` clears):

```json
{
  "kind": "set_form_field_max_length",
  "payload": { "name": "Name", "max_length": 32 }
}
```

Document `/Info` (Title / Author / Subject — omit keys to leave unchanged):

```json
{
  "kind": "set_document_metadata",
  "payload": {
    "title": "Updated Intake",
    "author": "Wave 4 Agent",
    "subject": "Onboarding refresh"
  }
}
```

Clear `/Info` Title, Author, and Subject (Creator/Producer left untouched):

```json
{
  "kind": "clear_document_metadata",
  "payload": {}
}
```

Rejected: encrypted PDFs, signed/certified PDFs, read-only fields, ambiguous radios
without an export value, empty metadata payloads.

Snapshots are opaque whole-file blobs (not OOXML part manifests).
