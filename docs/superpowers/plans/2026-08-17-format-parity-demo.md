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
