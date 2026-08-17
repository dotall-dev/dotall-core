---
name: pdf
description: >-
  Work with .pdf files through Dotall MCP. Use when inspecting pages, listing
  AcroForm fields, filling or clearing a text/checkbox/choice/radio field,
  inserting a sticky/text comment annotation, setting /Info metadata, or
  reverting a PDF in a Dotall workspace. Do not rewrite page content streams.
---

# PDF via Dotall MCP

v0 is **form fill + document metadata + comment annotations**. Do not unzip,
reprint, or rewrite page operators. Charts are N/A for PDF (`summary.charts` is
omit or `[]`).

## Workflow

```text
dotall_capabilities or dotall_inspect
  → dotall_read (page / field / full)
  → dotall_edit set_form_field | set_form_fields | set_form_field_readonly | set_form_field_required | set_form_field_multiline | set_form_field_password | set_form_field_max_length | set_form_field_comb | set_form_field_do_not_scroll | set_form_field_do_not_spell_check | set_form_field_rich_text | set_form_field_no_export | set_form_field_multi_select | set_form_field_combo | set_form_field_edit | clear_form_field | clear_all_form_fields | set_document_metadata | insert_comment (stage)
  → dotall_apply OR flush-on-close
  → dotall_history / revert
```

## Pack / needle-finding

Pack / needle-finding: call `dotall_search` (query like `10%` or `Rate`) instead
of unzipping OOXML. Then `read`/`edit` using returned selectors. `dotall viz` is
for humans inspecting `.all/`, not required in the edit loop.

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
Inspect surfaces `no_export` per field from `/Ff` bit 3.
Inspect surfaces `multi_select` per choice field from `/Ff` bit 20.
Inspect surfaces `combo` per choice field from `/Ff` bit 17.

Inspect `comments[]` lists non-Widget page annotations (`/Text`, `/FreeText`)
with `element_id`, `page`, `subtype`, `contents`, and `author`. AcroForm
`/Widget` annots are form fields, not comments. `charts` is omit or `[]`.

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

Mark a text field as comb (character boxes) or clear (`/Ff` Comb bit; `tx` only; typically paired with MaxLen):

```json
{
  "kind": "set_form_field_comb",
  "payload": { "name": "Name", "comb": true }
}
```

Mark a text field as do-not-scroll or clear (`/Ff` DoNotScroll bit; `tx` only; pairs with comb/max_length):

```json
{
  "kind": "set_form_field_do_not_scroll",
  "payload": { "name": "Name", "do_not_scroll": true }
}
```

Mark a text field as do-not-spell-check or clear (`/Ff` DoNotSpellCheck bit; `tx` only; pairs with do_not_scroll/comb):

```json
{
  "kind": "set_form_field_do_not_spell_check",
  "payload": { "name": "Name", "do_not_spell_check": true }
}
```

Mark a text field as rich-text or clear (`/Ff` RichText bit; `tx` only):

```json
{
  "kind": "set_form_field_rich_text",
  "payload": { "name": "Name", "rich_text": true }
}
```

Exclude a field from export or include it (`/Ff` NoExport bit; any field type):

```json
{
  "kind": "set_form_field_no_export",
  "payload": { "name": "Name", "no_export": true }
}
```

Mark a choice field as multi-select or single-select (`/Ff` MultiSelect bit; `ch` only):

```json
{
  "kind": "set_form_field_multi_select",
  "payload": { "name": "Department", "multi_select": true }
}
```

Mark a choice field as combo (dropdown) or list (`/Ff` Combo bit; `ch` only):

```json
{
  "kind": "set_form_field_combo",
  "payload": { "name": "Department", "combo": true }
}
```

Allow typing in a combo choice field (`/Ff` Edit bit; `ch` only):

```json
{
  "kind": "set_form_field_edit",
  "payload": { "name": "Department", "edit": true }
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

Insert a sticky/text comment annotation (new object only; no content-stream drawing):

```json
{
  "kind": "insert_comment",
  "payload": {
    "page": 1,
    "contents": "Check Name field",
    "author": "Dotall"
  }
}
```

`page` is 1-based. `author` defaults to `"Dotall"`. Creates `/Type /Annot`
`/Subtype /Text` with `/Contents` and `/T`, appends to the page `/Annots` array.
Does **not** rewrite page `/Contents` streams or mutate existing annot dicts
(including Widget form fields). Rejected: `set_comment`, `delete_comment`,
`replace_comment`, and any draw-text-on-page kind.

Rejected: encrypted PDFs, signed/certified PDFs, read-only fields, ambiguous radios
without an export value, empty metadata payloads.

Snapshots are opaque whole-file blobs (not OOXML part manifests).
