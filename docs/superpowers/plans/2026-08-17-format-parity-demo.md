# Format Parity Demo Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Independent formats may run as parallel Task subagents; verify each wave before starting the next for that format.

**Goal:** Close the highest-value gaps so agents can do roughly what they'd do with openpyxl / python-pptx / python-docx / pypdf-style tools via Dotall `FormatHandler` + CLI/MCP, enough for one strong multi-format demo PR.

**Architecture:** Keep the existing plug-and-play format crates. Prefer thin vertical slices (read selectors + one surgical edit op) over speculative shared writers. Reuse `dotall-ooxml` for ZIP packages; PDF stays opaque whole-file snapshots unless a slice clearly needs otherwise. Surgical OOXML patching remains the fidelity moat.

**Tech Stack:** Stable Rust workspace, `FormatHandler`, `zip` + `quick-xml` (Office), `lopdf` (PDF), blake3 snapshots, serde JSON models, CLI (`dotall-cli`) + stdio MCP (`dotall-mcp`).

**Branch:** `feat/format-parity-demo` (single PR; many commits; no amend; do not merge).

## Global Constraints

- Surgical OOXML: patch only target parts; untouched ZIP entries stay byte-identical.
- TDD: failing test first, then minimal implementation.
- No `unwrap()` in library code outside tests.
- `cargo fmt` and `cargo clippy -- -D warnings` clean for touched crates; periodically `--workspace`.
- Update `skills/*/SKILL.md` and `AGENTS.md` when new ops ship.
- Prefer demo-visible ops over architecture refactors.
- Charts/pivots/VBA/body PDF page rewrite remain preserve-only / out of scope.

---

## Audit snapshot (2026-08-17, `main` @ `8eab58f`)

### XLSX (`dotall-xlsx`) — strongest

| Area | Status |
|------|--------|
| Read | `full`, `sheet`, `range`, `ast_range`; inspect lists sheets, dims, named-range **names** |
| Edit | `set_cell_value`, `set_cell_formula`, `set_range`, `insert/delete_row/column`, `add/rename/delete_sheet` |
| Model | `merges`, `named_ranges` (name+formula), `style_table` (ids only), cell `style_id` / `number_format` |
| Gaps vs openpyxl | Merges not in read/inspect projections; named ranges not readable as a selector; no merge/unmerge edit; no style apply; no row/col dimension write; no freeze panes / data validation / comments |

### PPTX (`dotall-pptx`) — text-only v0

| Area | Status |
|------|--------|
| Read | `full`, `slide`, `notes` |
| Edit | `set_shape_text` only (text frames; rejects graphicFrame/SmartArt/charts/groups) |
| Model | slides → shapes `{name,text}`; notes string; media part list |
| Gaps vs python-pptx | No table shape read/edit; no add/delete/reorder slides; no add shape; no notes edit; no picture/placeholder ops |

### DOCX (`dotall-docx`) — body paragraphs only

| Area | Status |
|------|--------|
| Read | `full`, `paragraphs`; inspect flags `skipped_tables` |
| Edit | `set_paragraph_text` (body only; rejects tracked changes / SDT / fields) |
| Model | paragraphs with optional `style_id`; tables skipped entirely |
| Gaps vs python-docx | No table cell paragraphs; no headers/footers; no run-level edit; no style apply; no section breaks |

### PDF (`dotall-pdf`) — form fill v0

| Area | Status |
|------|--------|
| Read | `full`, `page`, `field`; page text best-effort |
| Edit | `set_form_field` for `tx` / `ch`; rejects `btn`, encrypted, signed |
| Model | pages+text, fields, outline titles, encrypted flag — **no document metadata** (title/author/etc.) |
| Gaps vs pypdf | Checkbox/radio; richer choice options list; metadata inspect; page text quality; no designed-page body edit (intentional) |

---

## Gap prioritization vs reference libraries

### Must-have (strong multi-format demo)

Enough for one CLI walkthrough that feels “agent-complete” across four formats:

1. **XLSX** — expose merges + named ranges in inspect/read (data already in model); keep existing structural + `set_range` working; smoke multi-sheet + range edit.
2. **PPTX** — `set_shape_text` (exists) **plus** either (A) read/edit table cell text on slides **or** (B) `add_slide` / `delete_slide` if surgical and safe. Prefer (A) if (B) risks presentation.xml + rels complexity blocking the demo.
3. **DOCX** — `set_paragraph_text` (exists) **plus** table cell paragraph read/edit (flip `skipped_tables` into real coverage for editable cells).
4. **PDF** — `set_form_field` (exists) **plus** checkbox (`btn`) when export values are clear **or** document metadata in inspect + field options for choice fields.

### Should-have (next waves on same PR if time)

| Format | Wave |
|--------|------|
| XLSX | `merge_cells` / `unmerge_cells` edit; `named_range` / `merges` read selectors; light style inspect (number format already on cells) |
| PPTX | `add_slide` (blank layout clone), `delete_slide`, `set_notes_text` |
| DOCX | header/footer paragraph read + `set_header_paragraph_text`; multi-run preserve richer than first-run clone |
| PDF | choice field option list in inspect; better page text extraction; `/Info` metadata |

### Explicit non-goals (this PR)

- Full Excel style engine, themes, conditional formatting writer, VBA, chart/pivot **mutation**
- Formula evaluation engine
- PowerPoint SmartArt/chart editing, freeform drawing, animations, theme rewrite
- Word tracked-changes authoring, content-control authoring, mail merge, full styles.xml rewrite
- PDF page content-stream / designed-page body edit, PDF/A certification, digital signature creation
- Cross-format unified IR, global `~/.all/cache`, watch daemon, multi-agent locking
- Python/JS SDKs

---

## File map (expected touch points)

| Format | Primary files |
|--------|----------------|
| XLSX | `crates/dotall-xlsx/src/{format,projection,selector,model}.rs`, `tests/read_contract.rs`, optionally `edits/` for merge ops |
| PPTX | `crates/dotall-pptx/src/{parser,model,projection,edits,format}.rs`, `tests/{read_contract,surgical_edit}.rs` |
| DOCX | `crates/dotall-docx/src/{parser,model,edits,projection,format}.rs`, `tests/{read_contract,surgical_edit}.rs` |
| PDF | `crates/dotall-pdf/src/{parser,model,edits,format,projection}.rs`, `tests/{read_contract,form_edit}.rs` |
| Docs | `skills/{xlsx,pptx,docx,pdf}/SKILL.md`, `AGENTS.md`, `demo/README.md` (extend multi-format smoke) |

Shared: only touch `dotall-ooxml` / `dotall-core` / CLI/MCP if a new op needs generic CLI flags — prefer format-owned `--ops-json` / MCP payloads.

---

## Wave 0 — Demo harness (shared)

### Task 0: Multi-format CLI smoke script + fixtures note

**Files:**
- Create or update: `demo/README.md` (add pptx/docx/pdf sections once fixtures exist under `demo/` or `/tmp`)
- Prefer generating fixtures in tests (existing pattern); smoke commands may copy test fixtures to `/tmp/dotall-parity-*`

**Acceptance:**
- Documented commands for all four formats: `init` → `inspect` → `read` → `edit` → `apply` → `history`
- No requirement to commit binary fixtures if tests generate them

- [ ] **Step 1:** After each format wave lands, append a smoke block to `demo/README.md` with exact CLI args.
- [ ] **Step 2:** Run smoke in `/tmp` and paste expected outcomes into the PR description over time.

---

## Wave 1 — XLSX must-have: surface merges + named ranges

### Task X1: Inspect + read expose merges and named-range formulas

**Files:**
- Modify: `crates/dotall-xlsx/src/format.rs` (`inspect` summary)
- Modify: `crates/dotall-xlsx/src/selector.rs`, `projection.rs` (optional selectors `merges`, `named_ranges`)
- Test: `crates/dotall-xlsx/tests/read_contract.rs`

**Interfaces:**
- Consumes: `WorkbookModel.merges`, `NamedRange { name, formula }`
- Produces: inspect JSON includes `merges` per sheet and named ranges as `{name, formula}`; optional `ReadSelector` kinds `merges` / `named_ranges`

**TDD acceptance:**
1. Fixture workbook with merge `A1:B2` and defined name `Rate` → `Inputs!$B$2`.
2. `inspect` summary contains that merge and `{ "name": "Rate", "formula": "..." }`.
3. `read` with `selector_kind=named_ranges` (or documented equivalent) returns the formula text.
4. Untouched: existing `range` / `set_range` / structural tests still pass.

- [ ] **Step 1: Write failing test** in `read_contract.rs` asserting inspect includes merge + named range formula.
- [ ] **Step 2: Run** `cargo test -p dotall-xlsx --test read_contract …` → FAIL.
- [ ] **Step 3: Implement** inspect fields + selector/projection as needed (no schema bump if additive JSON only; bump `SCHEMA_VERSION` only if artifact payload shape changes required — prefer inspect/projection-only).
- [ ] **Step 4: Run tests** → PASS; `cargo clippy -p dotall-xlsx -- -D warnings`.
- [ ] **Step 5: Update** `skills/xlsx/SKILL.md` selectors note; commit `feat(xlsx): expose merges and named ranges in inspect/read`.

### Task X2: CLI smoke multi-sheet + set_range

**Acceptance:**
```bash
# /tmp demo dir with a multi-sheet xlsx (or demo/financials.xlsx)
dotall inspect …  # shows ≥2 sheets + named_ranges/merges when present
dotall read … --range 'Sheet1!A1:B2'
dotall edit … --ops-json '[{"kind":"set_range","payload":{...}}]'
dotall apply … --all
```

- [ ] **Step 1:** Run smoke; fix regressions if structural/set_range broken.
- [ ] **Step 2:** Commit only if code fixes needed.

### Task X3 (should-have): `merge_cells` / `unmerge_cells` edit

**Files:** `crates/dotall-xlsx/src/edits/{ops,validate,writer/structural}.rs`, tests

**TDD acceptance:**
- Op `merge_cells` with `{sheet, range}` adds `<mergeCell ref="…"/>`; surgical patch; other sheets byte-identical.
- Reject overlapping merges with clear error.
- `unmerge_cells` removes matching ref.

---

## Wave 1 — PPTX must-have: table cells OR slide add/delete

### Task P1 (preferred): Parse + read slide tables; `set_table_cell_text`

**Files:**
- Modify: `crates/dotall-pptx/src/{model,parser,projection,edits,format}.rs`
- Test: `crates/dotall-pptx/tests/{read_contract,surgical_edit}.rs`

**Interfaces:**
- Extend `ShapeModel` **or** add `TableModel` on `SlideModel` with cells `{row, col, text, element_id}`.
- New edit: `set_table_cell_text` payload `{ slide, shape|table, row, col, text }`.
- Patch only the slide part containing the table (`p:graphicFrame` → `a:tbl`); leave media byte-identical.

**TDD acceptance:**
1. Fixture slide with 2×2 table; read shows four cell texts.
2. Edit cell (0,1); re-parse shows new text; other shapes unchanged; other slides' ZIP entries byte-identical.
3. Reject out-of-range row/col.
4. Capabilities advertise the new op with example payload.
5. Update `skills/pptx/SKILL.md`.

- [ ] **Step 1: Failing tests** for parse/read + surgical edit.
- [ ] **Step 2: Implement** parser + writer (replace `a:t` in target cell similarly to shape text).
- [ ] **Step 3: Verify** `cargo test -p dotall-pptx`; clippy; CLI smoke.
- [ ] **Step 4: Commit** `feat(pptx): read and set slide table cell text`.

### Task P2 (alternate / should-have): `add_slide` / `delete_slide`

Only if P1 lands early or P1 is blocked.

**Constraints:** Must update `ppt/presentation.xml`, `[Content_Types].xml`, and relationships surgically; duplicate a blank slide part template from fixtures — do not regenerate the whole package.

**TDD acceptance:**
- `add_slide` after `Slide 1` increases slide count by 1; new slide readable; previous slide parts unchanged bytes.
- `delete_slide` removes last non-only slide; rejects deleting the sole slide.
- Snapshots remain lossless for untouched parts.

### Task P3 (should-have): `set_notes_text`

Patch notes slide part only.

---

## Wave 1 — DOCX must-have: table paragraphs

### Task D1: Include table cell paragraphs in model + `set_paragraph_text`

**Files:**
- Modify: `crates/dotall-docx/src/{parser,model,edits,format,projection}.rs`
- Test: `crates/dotall-docx/tests/{read_contract,surgical_edit}.rs`

**Interfaces:**
- Parse `w:tbl` → cell paragraphs with stable `element_id` / sequential `index` (document order including tables).
- Replace `skipped_tables: bool` with `table_count` and/or keep flag `false` when tables are modeled; document the change in skill.
- Reuse `set_paragraph_text` targeting table cell paragraphs by `index` or `element_id`.
- Still reject tracked changes / SDT / fields inside cells.

**TDD acceptance:**
1. Fixture with body para + 1×2 table; read lists 3 paragraphs (or body + table indices as designed — document clearly).
2. Edit table cell paragraph; only `word/document.xml` changes; headers/media byte-identical.
3. Inspect no longer claims tables are skipped (or accurately reports modeled tables).
4. Update `skills/docx/SKILL.md`.

- [ ] **Step 1: Failing read + edit tests** with table fixture.
- [ ] **Step 2: Parser includes table paragraphs; writer finds target `w:p` in tables.**
- [ ] **Step 3: Verify** tests + clippy + CLI smoke.
- [ ] **Step 4: Commit** `feat(docx): read and edit table cell paragraphs`.

### Task D2 (should-have): Header/footer paragraphs

Parse `word/header*.xml` / `footer*.xml`; new op or same op with `{part: "header", index}` — prefer explicit `set_header_paragraph_text` for clarity.

---

## Wave 1 — PDF must-have: checkbox or metadata

### Task F1 (preferred): Checkbox / radio via `set_form_field`

**Files:**
- Modify: `crates/dotall-pdf/src/{edits,parser,model,format}.rs`
- Test: `crates/dotall-pdf/tests/form_edit.rs`, fixture with `/Btn`

**Interfaces:**
- For `field_type == "btn"`, accept values like `"On"` / `"Off"` / export name from `/AP` / `/V` / `/AS`.
- Store `export_values: Vec<String>` on `PdfFieldModel` when discoverable.
- Reject ambiguous radios without explicit export value.

**TDD acceptance:**
1. Fixture checkbox Off → set On → re-parse value On; history revert restores bytes.
2. Encrypted/signed still rejected.
3. Update `skills/pdf/SKILL.md`.

- [ ] **Step 1: Failing form_edit test** for checkbox.
- [ ] **Step 2: Implement** lopdf field update for `/Btn` appearance state.
- [ ] **Step 3: Verify** + commit `feat(pdf): set AcroForm checkbox fields`.

### Task F2 (alternate / parallel): Metadata in inspect

**Files:** `parser.rs`, `model.rs`, `format.rs` inspect summary

**TDD acceptance:**
- `/Info` dict fields `Title`, `Author`, `Subject`, `Creator`, `Producer` appear in inspect when present.
- No edit op required for must-have if F1 ships; metadata edit is should-have.

### Task F3 (should-have): Choice options list on inspect/read field

---

## Wave 2+ — Should-have backlog (same PR)

Execute in priority order when Wave 1 demo bar is met for all four formats:

1. X3 merge/unmerge edits
2. P2 slide add/delete
3. D2 headers/footers
4. P3 notes edit
5. F3 choice options + F2 if not done
6. XLSX `merges`/`named_ranges` dedicated selectors if only inspect was done in X1
7. Keep expanding demos as Wave 2+ ops land (standing demo gate above)

## Wave 3 — next demo-visible slices (parallel)

Execute after Wave 2 integrates (merge/unmerge, add/delete slide, headers/footers, PDF metadata/choice):

1. **PPTX P3:** `set_notes_text` — surgical notes slide part patch; update `demo/deck.pptx` with notes
2. **DOCX:** richer multi-run paragraph edit (preserve run breaks / clone rPr per run when safe) OR `insert_paragraph` after index — prefer multi-run fidelity on `set_paragraph_text`
3. **XLSX:** style-aware inspect/read — surface cell `number_format` / `style_id` in inspect or `ast_range` projection (no full style writer)
4. **PDF:** better page text extraction for `read.page` / full (still no designed-page body edit)

Each task ends with the standing demo gate (README + regenerate + CLI smoke new + prior paths).

## Wave 4 — next openpyxl-class gaps (parallel)

After Wave 3 integrates:

1. **XLSX:** surgical `set_column_width` and/or `set_row_height` (sheet dims write) — demo-visible layout control without a full style engine
2. **PPTX:** `move_slide` / reorder (or `duplicate_slide` if safer) — keep add/delete/notes working
3. **DOCX:** `insert_paragraph` after index (body; optionally table) — thin vertical slice
4. **PDF:** `set_document_metadata` for `/Info` Title/Author/Subject (edit path for Wave 2 inspect)

Each task ends with the standing demo gate.

## Wave 5 — next openpyxl-class gaps (parallel)

After Wave 4 integrates (col/row size, move_slide, insert_paragraph, set_document_metadata):

1. **XLSX:** `freeze_panes` read + surgical set (sheet view) — common openpyxl workflow
2. **PPTX:** `add_textbox` (or add simple text shape) on a slide — grow content beyond editing existing shapes
3. **DOCX:** `delete_paragraph` by index/element_id — pair with insert_paragraph
4. **PDF:** radio-group `Btn` support via `set_form_field` (export values) OR `clear_form_field` — prefer radios if fixture-friendly

Each task ends with the standing demo gate.

## Wave 6 — next openpyxl-class gaps (parallel)

After Wave 5 integrates (freeze_panes, add_textbox, delete_paragraph, radio Btn):

1. **XLSX:** `define_name` / update named range formula (surgical workbook.xml) — pairs with Wave 1 named-range read
2. **PPTX:** `delete_shape` by slide+name — pairs with add_textbox
3. **DOCX:** `set_paragraph_style` (set `w:pStyle` / style_id) — light style apply without full styles.xml rewrite
4. **PDF:** `clear_form_field` (blank text / Off checkbox / clear choice) — common form workflow

Each task ends with the standing demo gate.

## Wave 7 — next openpyxl-class gaps (parallel)

After Wave 6 integrates (define_name, delete_shape, set_paragraph_style, clear_form_field):

1. **XLSX:** `delete_name` — remove a workbook defined name (surgical workbook.xml); pairs with `define_name`
2. **PPTX:** `rename_shape` by slide+old name → new name (surgical `cNvPr` on slide part); pairs with add/delete shape
3. **DOCX:** `set_paragraph_alignment` (`w:jc` left/center/right/both) — light para formatting beside style_id
4. **PDF:** `clear_document_metadata` — clear `/Info` Title/Author/Subject (opaque whole-file snapshot); pairs with `set_document_metadata`

Each task ends with the standing demo gate.

## Wave 8 — next openpyxl-class gaps (parallel)

After Wave 7 integrates (delete_name, rename_shape, set_paragraph_alignment, clear_document_metadata):

1. **XLSX:** `hide_sheet` — set/clear workbook sheet `state="hidden"` (surgical workbook.xml); reject hiding the last visible sheet
2. **PPTX:** `set_shape_bold` — set/clear bold on shape text runs (`a:rPr b`) inside a slide shape; content-on-slide (prefer over structural slide clone/reorder)
3. **DOCX:** `set_paragraph_bold` — set/clear `w:b` on runs in a body/table paragraph by index/element_id; content-in-document
4. **PDF:** `clear_all_form_fields` — clear every non-read-only AcroForm field (blank text/choice, Off buttons); pairs with `clear_form_field`

**Content preference (Wave 8+):** Prefer ops that edit text/runs/formatting **inside** slides and **inside** document body over structural/meta packaging (slide add/delete/duplicate/reorder, rename-only). Deprioritize further `duplicate_slide` / `move_slide`-class work unless needed for demos. Next PPTX/DOCX waves should grow run props (italic/underline/size/font), hyperlinks, list/bullet text, replace-across-shapes, richer table cell content.

Each task ends with the standing demo gate.

## Wave 9 — content-first run props + companion slices (parallel)

After Wave 8 integrates (hide_sheet, set_shape_bold, set_paragraph_bold, clear_all_form_fields; duplicate_slide deferred):

1. **XLSX:** `set_tab_color` — set/clear worksheet tab color via surgical `sheetPr`/`tabColor` (`rgb` AARRGGBB or clear); inspect surfaces `tab_color` when present
2. **PPTX:** `set_shape_italic` — set/clear italic on shape text runs (`a:rPr i`); content-on-slide companion to `set_shape_bold`
3. **DOCX:** `set_paragraph_italic` — set/clear `w:i` on runs in a body/table paragraph; content-in-document companion to `set_paragraph_bold`
4. **PDF:** `set_form_fields` — bulk set multiple AcroForm fields in one op (name→value map); pairs with `set_form_field` / `clear_all_form_fields`

**Deferred:** `duplicate_slide` / further structural PPTX packaging; underline/size/font run props land in later waves.

Each task ends with the standing demo gate.

## Wave 10 — underline run props + companion slices (parallel)

After Wave 9 integrates (set_tab_color, set_shape_italic, set_paragraph_italic, set_form_fields):

1. **XLSX:** `set_auto_filter` — set/clear worksheet `<autoFilter ref="…"/>` (A1-style range or clear); inspect surfaces `auto_filter` when present
2. **PPTX:** `set_shape_underline` — set/clear underline on shape text runs (`a:rPr u="sng"` / `u="none"`); content-on-slide companion to bold/italic
3. **DOCX:** `set_paragraph_underline` — set/clear `w:u` on runs (`w:val="single"` / `none`); content-in-document companion to bold/italic
4. **PDF:** `set_form_field_readonly` — set/clear AcroForm field ReadOnly (`/Ff` bit 1); pairs with form fill workflow

**Deferred:** font size/name, highlight, replace-across-shapes/paragraphs; further structural PPTX packaging.

Each task ends with the standing demo gate.

## Wave 11 — font size run props + companion slices (parallel)

After Wave 10 integrates (set_auto_filter, set_shape_underline, set_paragraph_underline, set_form_field_readonly):

1. **XLSX:** `set_print_area` — set/clear worksheet print area via surgical workbook `_xlnm.Print_Area` defined name (A1-style range or clear); inspect surfaces `print_area` when present
2. **PPTX:** `set_shape_font_size` — set/clear font size on shape text runs (`a:rPr sz` in hundredths of a point from `size_pt`); content-on-slide companion to bold/italic/underline
3. **DOCX:** `set_paragraph_font_size` — set/clear `w:sz` / `w:szCs` on runs (half-points from `size_pt`); content-in-document companion to bold/italic/underline
4. **PDF:** `set_form_field_required` — set/clear AcroForm field Required (`/Ff` bit 2); pairs with `set_form_field_readonly`

**Deferred:** font name, highlight/color, replace-across-shapes/paragraphs; further structural PPTX packaging.

Each task ends with the standing demo gate.

## Wave 12 — font name run props + companion slices (parallel)

After Wave 11 integrates (set_print_area, set_shape_font_size, set_paragraph_font_size, set_form_field_required):

1. **XLSX:** `set_print_titles` — set/clear worksheet print titles via surgical workbook `_xlnm.Print_Titles` (`rows` like `1:1` and/or `cols` like `A:A`; both null clears); inspect surfaces `print_titles` when present
2. **PPTX:** `set_shape_font_name` — set/clear font name on shape text runs (`a:latin` / `a:ea` / `a:cs` `typeface` inside `a:rPr`); content-on-slide companion to font size
3. **DOCX:** `set_paragraph_font_name` — set/clear `w:rFonts` on runs (`w:ascii` / `w:hAnsi` / `w:cs` from `font`); content-in-document companion to font size
4. **PDF:** `set_form_field_multiline` — set/clear AcroForm text-field Multiline (`/Ff` bit 13); pairs with readonly/required; reject non-`tx` fields

**Deferred:** highlight/color, replace-across-shapes/paragraphs; further structural PPTX packaging.

Each task ends with the standing demo gate.

## Wave 13 — font color run props + companion slices (parallel)

After Wave 12 integrates (set_print_titles, set_shape_font_name, set_paragraph_font_name, set_form_field_multiline):

1. **XLSX:** `set_page_orientation` — set worksheet print orientation via surgical `pageSetup` (`orientation` = `portrait` | `landscape`); inspect surfaces `page_orientation` when present
2. **PPTX:** `set_shape_font_color` — set/clear solid sRGB text color on shape runs (`a:solidFill`/`a:srgbClr val` inside `a:rPr` from `#RRGGBB` / `RRGGBB`; null clears); content-on-slide companion to font name/size
3. **DOCX:** `set_paragraph_font_color` — set/clear `w:color` on runs (`w:val` hex from `#RRGGBB` / `RRGGBB`; null clears); content-in-document companion to font name/size
4. **PDF:** `set_form_field_password` — set/clear AcroForm text-field Password (`/Ff` bit 14); pairs with multiline/readonly/required; reject non-`tx` fields

**Deferred:** highlight/fill, replace-across-shapes/paragraphs; further structural PPTX packaging.

Each task ends with the standing demo gate.

## Wave 14 — highlight run props + companion slices (parallel)

After Wave 13 integrates (set_page_orientation, set_shape_font_color, set_paragraph_font_color, set_form_field_password):

1. **XLSX:** `set_page_margins` — set worksheet print margins via surgical `pageMargins` (`left`/`right`/`top`/`bottom` in inches; optional `header`/`footer`); inspect surfaces `page_margins` when present
2. **PPTX:** `set_shape_highlight` — set/clear text highlight on shape runs (`a:highlight`/`a:srgbClr val` inside `a:rPr` from `#RRGGBB` / `RRGGBB`; null clears); content-on-slide companion to font color
3. **DOCX:** `set_paragraph_highlight` — set/clear `w:highlight` on runs (`w:val` from Word highlight names e.g. `yellow` / `none`; null clears); content-in-document companion to font color
4. **PDF:** `set_form_field_max_length` — set/clear AcroForm text-field `/MaxLen` (positive integer or null); pairs with password/multiline; reject non-`tx` fields

**Deferred:** replace-across-shapes/paragraphs, hyperlinks, richer table cell content; further structural PPTX packaging.

Each task ends with the standing demo gate.

## Wave 15 — replace text (content-first) + companion slices (parallel)

After Wave 14 integrates (set_page_margins, set_shape_highlight, set_paragraph_highlight, set_form_field_max_length):

1. **XLSX:** `set_print_scale` — set worksheet print scale via surgical `pageSetup` (`scale` integer 10–400); inspect surfaces `print_scale` when present
2. **PPTX:** `replace_shape_text` — find/replace substring inside one slide shape’s text (payload `slide`, `shape`, `find`, `replace`; reject empty `find` / no match); content-on-slide; reuses surgical shape text patch (first-run rewrite like `set_shape_text`)
3. **DOCX:** `replace_paragraph_text` — find/replace substring inside one body/table paragraph by index/element_id (`find`, `replace`; reject empty `find` / no match); content-in-document companion
4. **PDF:** `set_form_field_comb` — set/clear AcroForm text-field Comb (`/Ff` bit 25); pairs with `set_form_field_max_length`; reject non-`tx` fields; inspect surfaces `comb`

**Deferred:** slide-/document-wide replace-across, hyperlinks, richer table cell content, strike/superscript run props; further structural PPTX packaging.

Each task ends with the standing demo gate.

## Wave 16 — strikethrough run props + companion slices (parallel)

After Wave 15 integrates (set_print_scale, replace_shape_text, replace_paragraph_text, set_form_field_comb):

1. **XLSX:** `set_fit_to_page` — set/clear worksheet fit-to-page via surgical `pageSetup` `fitToWidth`/`fitToHeight` and `sheetPr`/`pageSetUpPr` `fitToPage` (`width`/`height` non-negative integers; both null clears); inspect surfaces `fit_to_page` when present
2. **PPTX:** `set_shape_strikethrough` — set/clear strikethrough on shape text runs (`a:rPr strike="sngStrike"` / `strike="noStrike"`); content-on-slide companion to highlight/underline
3. **DOCX:** `set_paragraph_strikethrough` — set/clear `w:strike` on runs in a body/table paragraph; content-in-document companion
4. **PDF:** `set_form_field_do_not_scroll` — set/clear AcroForm text-field DoNotScroll (`/Ff` bit 24); pairs with comb/max_length; reject non-`tx` fields; inspect surfaces `do_not_scroll`

**Deferred:** superscript/subscript, hyperlinks, richer table cell content, list/bullet text, document-/slide-wide replace-across; further structural PPTX packaging.

Each task ends with the standing demo gate.

## Wave 17 — superscript/subscript (content-first) + companion slices (parallel)

After Wave 16 integrates (set_fit_to_page, set_shape_strikethrough, set_paragraph_strikethrough, set_form_field_do_not_scroll):

1. **XLSX:** `set_center_on_page` — set/clear worksheet print centering via surgical `printOptions` `horizontalCentered`/`verticalCentered` (`horizontal`/`vertical` booleans; both false clears); inspect surfaces `center_on_page` when present
2. **PPTX:** `set_shape_vert_align` — set/clear superscript/subscript on shape text runs (`a:rPr baseline="30000"` / `baseline="-25000"`; null clears attribute); content-on-slide companion to strikethrough (`vert_align`: `superscript` | `subscript` | null)
3. **DOCX:** `set_paragraph_vert_align` — set/clear `w:vertAlign` on runs (`w:val="superscript"` / `subscript`; null clears); content-in-document companion
4. **PDF:** `set_form_field_do_not_spell_check` — set/clear AcroForm text-field DoNotSpellCheck (`/Ff` bit 23); pairs with do_not_scroll/comb; reject non-`tx` fields; inspect surfaces `do_not_spell_check`

**Deferred:** hyperlinks, richer table cell content, list/bullet text, document-/slide-wide replace-across; further structural PPTX packaging.

Each task ends with the standing demo gate.

---

## Standing requirement — end every format wave with demos

After **each** format wave completes (Wave 1 and every future wave):

1. **Improve `demo/`** for that format with realistic examples (not only tiny test stubs):
   - Expand `demo/README.md` with a short CLI walkthrough per touched format.
   - Add/update demo files under `demo/` (`financials.xlsx`, `deck.pptx`, `memo.docx`,
     `form.pdf`, …) that look like real agent workloads.
   - Prefer regenerating via `cargo run -p dotall-cli --example generate_demos`
     (crate `demo_*` fixtures + rust_xlsxwriter). Keep binaries small; document generation.
   - Wire MCP example / skill pointers so agents know the demo paths.
2. **Verify nothing broke**
   - `cargo test -p <touched crates>`; periodically `--workspace` + clippy `-D warnings`
   - CLI smoke on BOTH the wave’s new capability AND prior v0 paths using `demo/` files
     (xlsx `set_cell_value`/read/deps; pptx `set_shape_text`; docx `set_paragraph_text`;
     pdf `set_form_field`)
   - Fix any broken demo or test before starting the next wave
3. Commit demo + README updates on `feat/format-parity-demo`; push; **do not merge**.

## Verification gates (every wave)

```bash
cargo test -p <touched-crate>
cargo clippy -p <touched-crate> -- -D warnings
# periodically:
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --check
cargo run -p dotall-cli --example generate_demos
# then CLI smoke from demo/README.md
```

CLI smoke uses committed files under `demo/` (preferred) or `/tmp` copies.

---

## Demo bar checklist (claim “Wave 1 done” only when all true)

- [ ] **XLSX:** multi-sheet read + `set_range` apply works; merges and/or named ranges visible in inspect/read
- [ ] **PPTX:** `set_shape_text` + (table cell text **or** add/delete slide) end-to-end via CLI
- [ ] **DOCX:** `set_paragraph_text` on a **table cell** paragraph end-to-end via CLI
- [ ] **PDF:** `set_form_field` + (checkbox **or** metadata inspect) end-to-end via CLI
- [ ] Skills updated for each shipped op
- [ ] `cargo test --workspace` green

---

## Orchestration notes

- Launch parallel Task subagents for XLSX / PPTX / DOCX / PDF Wave 1 tasks when independent.
- After each returns: run verification gates for that crate; commit; push to `feat/format-parity-demo`.
- If one format blocks, continue others.
- Do **not** merge the PR; do **not** amend commits.
- When stopping: report PR URL, this plan path, shipped vs left, demo commands.

## Spec coverage self-check

| Requirement | Task |
|-------------|------|
| Audit + gaps | This document (Phase 0) |
| XLSX merges/named ranges demo | X1, X2 |
| XLSX merge edit | X3 |
| PPTX table or slides | P1 / P2 |
| DOCX tables | D1 |
| DOCX headers | D2 |
| PDF checkbox or metadata | F1 / F2 |
| Non-goals listed | Explicit non-goals section |
| Skills/AGENTS updates | Each task commit checklist |
| Single PR continuous build | Branch + orchestration notes |
