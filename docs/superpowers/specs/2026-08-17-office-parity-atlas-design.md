# Office-Parity Atlas Design

**Date:** 2026-08-17  
**Status:** Approved for planning  
**Related:**
- `docs/superpowers/plans/2026-08-17-format-parity-demo.md` (Waves 1–22 shipped; squash-merged as #15)
- `docs/superpowers/specs/2026-08-17-killer-demo-design.md` (search / viz / Q3 bake-off)
- `docs/superpowers/specs/2026-08-15-office-pdf-format-families-design.md` (format crates, no universal AST)
- `docs/specs/dotall-overview.md`
- `docs/specs/core-format-architecture.md`

This document locks the **next workstream after format-parity #15**. It is a
design spec, not an implementation plan. Wave 23+ ops are not started here.

## Goal

Give agents an **Office-parity atlas**: enough inspect/read/edit coverage that
a human (or agent) can do the same **jobs** they would do in Excel / PowerPoint /
Word / Preview *and* the same **must-ship** surface as openpyxl / python-pptx /
python-docx / pypdf — without becoming a full Office clone.

Priority stays the **Office suite** (xlsx → pptx → docx, then pdf). Surgical
OOXML / PDF patching remains the moat: patch only the parts this op owns;
untouched ZIP entries and PDF objects stay byte-identical.

## Locked decisions

| Topic | Choice |
|-------|--------|
| Bar | **Hybrid:** human jobs as jobs-to-be-done; OSS libraries as must-ship checklist; Office power features preserve-or-reject |
| Comments | Inspect/read + **insert new only**. No surgical edit/delete/replace of an existing comment |
| Pictures | Inspect/read + **insert new only**. No surgical edit/delete/replace of an existing image |
| Charts | Inspect/read only. **No insert, no mutate** |
| Insert mechanics | Additive ZIP/PDF (new parts + rels / new annotation objects). Existing comment/picture mutate is rejected |
| PDF stamps | Annotation objects, **not** content-stream rewrite |
| IR | **Format-owned models** in `dotall-xlsx` / `dotall-pptx` / `dotall-docx` / `dotall-pdf`. No universal Office AST / unified IR |
| Inspect growth | Per-format `comments[]`, `pictures[]`, `charts[]` (charts may be empty/omitted on PDF) |
| Delivery | Parallel xlsx/pptx/docx/pdf waves, same cadence as Waves 1–22 |
| First waves | **23** comment the pack · **24** stamp a logo · **25** look like a report |
| Formula engine | Out of scope (agent quotes cells it `read`; Excel recalc is native) |

Do not re-litigate this table.

## 1. Jobs-to-be-done (human workflows)

Each job is a **demo-visible** outcome on the Q3 pack (or `demo/` smoke files).
Ops are format-owned; names below are intent, not frozen payload schemas —
discover via `capabilities` at implementation time.

### XLSX — review a workbook without breaking it

| Job | What the agent does | First wave |
|-----|---------------------|------------|
| Comment the pack | Thread a review note on a live cell (e.g. `Rate`) so Excel shows a comment | 23 |
| Stamp a logo | Insert a new picture (header/logo) onto a sheet; do not retouch existing drawings | 24 |
| Look like a report | Apply cell **font** and **fill** so the board pack reads as formatted, not raw | 25 |
| Read the picture/chart inventory | Inspect lists comments, pictures, charts without unzipping | 23–24 |

### PPTX — mark a deck, brand it, polish a table

| Job | What the agent does | First wave |
|-----|---------------------|------------|
| Comment the pack | Insert a new comment on a slide (or shape) that PowerPoint shows | 23 |
| Stamp a logo | Insert a new picture on a slide; leave existing media byte-identical | 24 |
| Look like a report | Table-cell **italic** or **fill** so a KPI table looks designed | 25 |
| Chart inventory | Inspect/read chart presence and titles/series labels; never rewrite chart XML | 23+ |

### DOCX — review a memo, brand it, paginate it

| Job | What the agent does | First wave |
|-----|---------------------|------------|
| Comment the pack | Insert a new Word comment anchored to a paragraph | 23 |
| Stamp a logo | Insert a new inline picture; do not replace an existing image | 24 |
| Look like a report | Paragraph **spacing** and/or a **page break** so the memo paginates like a briefing | 25 |

### PDF — annotate a form, stamp, rotate

| Job | What the agent does | First wave |
|-----|---------------------|------------|
| Comment the pack | Insert a new text/sticky annotation (comment), not a content-stream rewrite | 23 |
| Stamp a logo | Insert a stamp/image **annotation**; do not draw into page content streams | 24 |
| Look like a report | `rotate_page` so a scanned page sits upright | 25 |

## 2. OSS must-ship checklist

Libraries are a **coverage checklist**, not an API to clone. “Must-ship” means
inspect/read plus the insert/style ops locked above — not every method on the
Python object.

### openpyxl (`dotall-xlsx`)

| Library surface | After #15 | Atlas |
|-----------------|-----------|-------|
| Cells / formulas / sheets / merge / names | Shipped | Keep |
| Row/col size, freeze, hide, tab color, autofilter, print setup | Shipped | Keep |
| `Comment` on a cell | Missing | Inspect + **insert new** (Wave 23). Reject edit/delete/replace of existing |
| `openpyxl.drawing.image.Image` | Missing | Inspect + **insert new** (Wave 24). Reject mutate existing |
| `openpyxl.chart.*` | Preserve-only (`summary.preserved`) | Inspect/read chart list (title, sheet, anchor if cheap). **No insert/mutate** |
| `Font` / `PatternFill` on a cell | `style_id` ids only | Wave 25: surgical cell font + fill |
| Borders, number-format writer, protection, data validation, conditional formatting, VBA, pivot mutate | — | Preserve-or-reject (not Waves 23–25) |

### python-pptx (`dotall-pptx`)

| Library surface | After #15 | Atlas |
|-----------------|-----------|-------|
| Shape/table text, notes, add/delete/move slide, textbox, run styles, hyperlink, bullets | Shipped | Keep |
| Slide/shape comments | Missing | Inspect + **insert new** (Wave 23) |
| Picture shapes (`slide.shapes.add_picture`) | Media listed as unmodeled | Inspect + **insert new** (Wave 24) |
| Charts | Rejected on text ops | Inspect/read only. No insert/mutate |
| Table cell italic / fill | Bold shipped | Wave 25: italic **or** fill (one is enough for the job) |
| SmartArt, animations, theme rewrite, freeform | — | Preserve-or-reject |

### python-docx (`dotall-docx`)

| Library surface | After #15 | Atlas |
|-----------------|-----------|-------|
| Body + table paragraphs, headers/footers, run styles, bullets, cell shading | Shipped | Keep |
| Comments | Listed/unmodeled in v0 families spec | Inspect + **insert new** (Wave 23) |
| Inline pictures | Missing | Inspect + **insert new** (Wave 24) |
| Paragraph spacing / page break | Missing | Wave 25 |
| Track-changes authoring, mail merge, content-control authoring, styles.xml rewrite | — | Preserve-or-reject |

### pypdf (`dotall-pdf`)

| Library surface | After #15 | Atlas |
|-----------------|-----------|-------|
| Form fill (tx/ch/btn), field flags, `/Info` metadata | Shipped | Keep |
| Annotations (comments / stamps) | Missing | Inspect + **insert new** comment (23) and stamp (24) as annotation objects |
| Page rotate | Missing | Wave 25 `rotate_page` |
| Page content-stream drawing, PDF/A, signature **creation** | — | Preserve-or-reject. Encrypted/certified-signed files still reject |

## 3. Waves 23–25

Same delivery shape as Waves 1–22: one vertical slice per format, parallel
crates, TDD, skills + demo smoke, surgical golden tests. Do **not** start these
ops in the spec PR.

### Wave 23 — Comment the pack (all four formats)

**Outcome:** agent can `inspect` comments and insert **one new comment** per
file on the Q3 pack (or demo fixtures). Native apps show the note.

| Format | Inspect | Insert (additive) | Reject |
|--------|---------|-------------------|--------|
| xlsx | `comments[]` — sheet, cell, author, text, `element_id` | New comment on an existing cell | Edit/delete/replace existing `commentN.xml` / vml |
| pptx | `comments[]` — slide, optional shape, author, text, `element_id` | New comment on a slide | Mutate existing comment parts |
| docx | `comments[]` — paragraph `element_id`, author, text | New comment anchored to a paragraph | Mutate `word/comments.xml` entries in place |
| pdf | `comments[]` — page, subtype, contents, `element_id` | New text/sticky annotation | Rewrite page content streams; mutate existing annots |

Read selectors may add `comments` when a projection is cheaper than stuffing
the full inspect blob.

### Wave 24 — Stamp a logo (read + insert pictures)

**Outcome:** agent can list pictures and insert **one new image** (logo/stamp).
Existing media stays byte-identical.

| Format | Inspect | Insert | Reject |
|--------|---------|--------|--------|
| xlsx | `pictures[]` — sheet, name/anchor, content-type, `element_id` | New drawing + media part | Replace/crop/delete existing drawing |
| pptx | `pictures[]` — slide, name, media part, `element_id` | New `p:pic` + media + rels | Mutate existing `ppt/media/*` |
| docx | `pictures[]` — paragraph, media part, `element_id` | New inline drawing + media | Mutate existing `word/media/*` |
| pdf | `pictures[]` / stamp annots — page, subtype, `element_id` | New stamp/image **annotation** | Content-stream XObject rewrite |

Charts stay **out of this wave** except remaining inspect/read work if Wave 23
did not land chart arrays.

### Wave 25 — Look like a report (light style / layout)

**Outcome:** the four files look formatted, not just text-correct.

| Format | Op (intent) | Notes |
|--------|-------------|-------|
| xlsx | Cell **font** and **fill** | Surgical `styles.xml` + cell `s=` as needed; no theme rewrite |
| pptx | Table cell **italic** or **fill** | One of the two is sufficient; prefer italic if fill fights tblStyle |
| docx | Paragraph **spacing** and/or **page break** | `w:spacing` / `w:br w:type="page"` (or equivalent); not a styles.xml rewrite |
| pdf | `rotate_page` | Page `/Rotate`; not content-stream redraw |

Later waves (26+) may continue the OSS checklist (xlsx number format write,
pptx picture position, docx numbering, pdf annotation reply, …) but are **not**
locked here.

## 4. Architecture

### Format-owned IR (no unified Office AST)

Keep independent crates and envelopes:

```text
xlsx.workbook     → WorkbookModel
pptx.presentation → PresentationModel
docx.document     → DocumentModel
pdf.document      → PdfModel
```

Engine / MCP / CLI stay format-agnostic. Inspect JSON **may** grow parallel
arrays per format:

```json
{
  "comments": [{ "element_id": "…", "author": "…", "text": "…", "anchor": "…" }],
  "pictures": [{ "element_id": "…", "part": "…", "anchor": "…" }],
  "charts":   [{ "element_id": "…", "title": "…", "anchor": "…" }]
}
```

Field names inside each object are format-owned (`sheet` vs `slide` vs
`paragraph` vs `page`). Do **not** introduce a cross-format `OfficeNode`.
`charts[]` is omit-or-empty on PDF.

### Insert-only comments and pictures

- **Insert** allocates a new `element_id`, new ZIP part(s) and relationships
  (Office) or a new annotation object (PDF).
- **Validate** rejects ops that target an existing comment/picture `element_id`
  for edit, delete, or replace — including “set comment text” and “swap image
  bytes.”
- Existing comment/picture XML and media bytes stay byte-identical on every
  unrelated edit (same moat as today).

### Charts: inspect/read only

- Parse chart parts / graphic frames enough to list them for agents (so they
  stop unzipping `xl/charts`).
- Any edit that would rewrite chart XML, series formulas, or chart drawings
  **rejects** with `UnsupportedCapability`.
- Structural xlsx ops that already reject inbound chart references keep that
  behavior.

### PDF stamps vs page drawing

Stamps and comments are **annotation dictionaries** (+ optional appearance
streams owned by that annot). Wave 24 must not rewrite page content streams to
“draw a logo.” Encrypted or certified-signed files continue to reject.

## 5. Non-goals

Preserve-or-reject (do not author; do not silently corrupt):

- Charts / pivots / VBA **mutation**
- SmartArt / animations / theme rewrite
- Word track-changes authoring / mail merge / content-control authoring
- PDF page content-stream drawing, PDF/A, signature **creation**
- Formula evaluation engine
- Unified cross-format IR / universal Office AST
- Surgical edit/delete/replace of **existing** comments or pictures
- Chart **insert**
- Python/JS SDKs, global `~/.all/cache`, watch daemon, multi-agent locking

These stay out even if an OSS library exposes them.

## 6. Testing

Same bar as format families + format-parity:

- **Inspect:** fixture with one comment, one picture, and (Office) one chart →
  arrays populated; files without them omit or return `[]`.
- **Insert comment/picture:** semantic result visible on re-parse; **untouched
  ZIP entries byte-identical**; new parts appear only for the insert.
- **Reject:** edit/delete/replace existing comment or picture; chart mutate;
  PDF content-stream stamp.
- **Wave 25:** style/rotate result correct; unrelated parts byte-identical
  (xlsx `styles.xml` is an allowed target part when font/fill needs a new xf).
- **Round-trip:** `decode(encode(bytes)) == bytes` for snapshots; PDF annot
  insert preserves other objects.
- **Skills:** `skills/{xlsx,pptx,docx,pdf}/SKILL.md` describe insert-only and
  chart read-only.
- **Demo:** CLI smoke on `demo/` (or Q3 pack) for Waves 23–25; regenerate via
  `cargo run -p dotall-cli --example generate_demos` when fixtures need a
  comment/logo/chart.
- Gates: `cargo test --workspace`, `cargo fmt`, clippy `-D warnings`.

## 7. Related docs

| Doc | Role vs this atlas |
|-----|--------------------|
| `docs/superpowers/plans/2026-08-17-format-parity-demo.md` | Waves 1–22: agent-complete text/structure/print/form ops. **Done** (#15). This atlas starts at 23. |
| `docs/superpowers/specs/2026-08-17-killer-demo-design.md` | Investor bake-off: `dotall search`, `dotall viz`, Q3 pack. Viz UI follow-ups may land on this branch; they are not format ops. |
| `docs/superpowers/specs/2026-08-15-office-pdf-format-families-design.md` | Crate layout, `FormatHandler`, surgical ZIP, PDF form-fill v0. Atlas **extends** inspect/edit; it does not replace the families contract. |
| `docs/specs/core-format-architecture.md` | Plug-and-play handlers; still no universal model. |

Implementation plans for Waves 23–25 are a **follow-up** (writing-plans), not
this spec PR.
