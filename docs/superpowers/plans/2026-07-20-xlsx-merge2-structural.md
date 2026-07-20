# XLSX Merge 2 Structural Edits Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Full Merge 2 — impact/reject, `set_range`, row/col insert/delete, sheet add/rename/delete — on the Merge 1 transaction spine.

**Spec authority:** `docs/superpowers/specs/2026-07-20-xlsx-merge2-structural-design.md`  
**Supersedes task detail in:** `docs/superpowers/plans/2026-07-18-broaden-xlsx-edit-operations.md` (reuse task bodies; this file is the entrypoint + Merge 1 API anchors).

**Worktree:** `.worktrees/feat-xlsx-merge2-structural` on `feat/xlsx-merge2-structural` from `main`.

**Commit policy:** Per-task commits authorized when executing this plan.

**Safety rule:** Patch every understood reference, or reject before mutation with part + construct. Never silently leave stale refs. Charts/pivots/VBA = reject if mutation required.

---

## Merge 1 anchors (do not reinvent)

```text
crates/dotall-xlsx/src/edits/{ops,validate,mod}.rs
crates/dotall-xlsx/src/edits/writer/{mod,package,worksheet,shared_strings}.rs
crates/dotall-xlsx/src/dependencies/graph.rs
crates/dotall-xlsx/tests/surgical_edit.rs   # ZIP fidelity helper
crates/dotall-core Engine edit/apply/history
```

---

### Task 1: Impact inventory + reject unsupported

**Files:** `edits/impact.rs`, `registry` FormatDescriptor capabilities, `format.rs`, tests.

- [ ] **Step 1:** Add `EditCapability` to format descriptor (or xlsx-local capability list exposed via inspect). Advertise only proven cell ops initially (`set_cell_value`, `set_cell_formula`).
- [ ] **Step 2:** Implement `ImpactInventory` + `UnsupportedImpact` by reading package relationships/XML (not filename guesses).
- [ ] **Step 3:** Rejection tests: op intersecting pivot/chart source → validate fails with clear message; unrelated sheet edit allowed without touching pivot part.
  - Prefer minimal hand-crafted OOXML zip fixtures if rust_xlsxwriter cannot emit pivots.
- [ ] **Step 4:** Commit `feat(xlsx): inventory edit impact and reject unsupported parts`

---

### Task 2: Atomic `set_range`

**Files:** `ops.rs`, `validate.rs`, `writer/worksheet.rs`, `tests/set_range.rs`

- [ ] **Step 1:** `SetRange { sheet, start_cell, values: Vec<Vec<EditableCell>> }` with `EditableCell::{Value, Formula}`.
- [ ] **Step 2:** Validate: no empty/ragged rows; XLSX limits; no duplicate targets in tx; formula normalize; max 10_000 cells; semantic_diff per cell row-major; dependency_impact via graph.
- [ ] **Step 3:** Expand to cell ops; single worksheet patch pass (merge with shared strings path).
- [ ] **Step 4:** Tests + advertise `set_range`. Commit `feat(xlsx): set rectangular ranges`

---

### Task 3: Coordinate transformations (pure)

**Files:** `edits/transform/{mod,address,formula,sqref}.rs`

- [ ] Axis insert/delete transforms for cells/ranges; `#REF!` / Removed / Kept.
- [ ] Formula rewrite via lexer spans (extend lexer if needed); sheet-qualified rules.
- [ ] `sqref` list rewrite.
- [ ] Commit `feat(xlsx): pure coordinate and formula transforms`

---

### Task 4: Insert/delete rows

**Files:** ops/validate/writer + `tests/structural_rows_columns.rs`

- [ ] `InsertRow` / `DeleteRow`; reject on unsupported impact.
- [ ] Patch worksheet constructs listed in broaden plan Task 4; rewrite formulas across sheets; remove calcChain when present.
- [ ] Golden tests + fidelity. Commit `feat(xlsx): insert and delete rows`

---

### Task 5: Insert/delete columns

- [ ] `InsertColumn` / `DeleteColumn` via same `Axis` transforms.
- [ ] XFD boundary tests. Commit `feat(xlsx): insert and delete columns`

---

### Task 6: Add and rename sheets

- [ ] `AddSheet` / `RenameSheet` with Excel name rules.
- [ ] Surgical workbook/rels/content types (+ app.xml when present).
- [ ] Rename rewrites sheet-qualified refs we understand; reject otherwise.
- [ ] Commit `feat(xlsx): add and rename sheets`

---

### Task 7: Delete sheets

- [ ] `DeleteSheet` + `DeleteSheetPolicy::{RejectIfReferenced, ReplaceReferencesWithRefError}`.
- [ ] Reject deleting last visible sheet.
- [ ] Commit `feat(xlsx): safely delete sheets`

---

### Task 8: CLI + quality gate

- [ ] CLI `--op` for new ops; inspect advertises capabilities after tests pass.
- [ ] E2E per op family (stage/apply/history/revert).
- [ ] `cargo fmt --check && cargo test --workspace && cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] Commit `feat(cli): expose Merge 2 structural edit operations` / quality fixes as needed.
- [ ] Push PR to `main`.

---

## Detail reference

For expanded step text, fixtures lists, and exact error message examples, follow Tasks 1–8 in `2026-07-18-broaden-xlsx-edit-operations.md`, reconciled to Merge 1 writer APIs above.
