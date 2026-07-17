# Broader XLSX Edit Operations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend the proven transactional XLSX edit loop with `set_range`, row/column insertion and deletion, and sheet add/rename/delete while preserving or safely rejecting every affected OOXML reference.

**Architecture:** Reuse core transactions unchanged. Add typed operations and a coordinate-transform engine inside `dotall-xlsx`; each structural operation first inventories affected OOXML parts, computes a complete patch set, and rejects the edit before mutation when an affected construct is unsupported. Expand capabilities operation-by-operation only after golden fidelity tests pass.

**Tech Stack:** Stable Rust, existing XLSX writer, `quick-xml`, formula reference lexer/rewriter, ZIP raw-copy support, and golden OOXML fixtures.

---

## Preconditions and safety rule

Execute the transactional cell-edit plan first. Revise this plan against its actual
writer APIs.

Structural spreadsheet edits are not equivalent to moving XML nodes. They may
affect formulas, named ranges, merges, tables, filters, validations, conditional
formatting, drawings, chart series, print areas, hyperlinks, and calc metadata.

The non-negotiable rule is:

> Patch every affected reference that Dotall understands, or reject the operation
> before source mutation with the exact unsupported construct and part path.

Never preserve a stale reference silently.

Commit steps require explicit user authorization.

## Target files

```text
crates/dotall-xlsx/src/edits/
├── impact.rs
├── ops.rs
├── transform/
│   ├── address.rs
│   ├── formula.rs
│   ├── mod.rs
│   └── sqref.rs
├── validate.rs
└── writer/
    ├── package.rs
    ├── workbook.rs
    └── worksheet.rs
crates/dotall-xlsx/tests/
├── set_range.rs
├── structural_rows_columns.rs
└── structural_sheets.rs
fixtures/xlsx/structural/
```

### Task 1: Add operation capability descriptors and impact inventory

**Files:**

- Modify: `crates/dotall-core/src/registry/types.rs`
- Modify: `crates/dotall-xlsx/src/format.rs`
- Create: `crates/dotall-xlsx/src/edits/impact.rs`
- Modify: `crates/dotall-xlsx/src/edits/mod.rs`

- [ ] **Step 1: Make edit capabilities discoverable**

Add:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditCapability {
    pub operation: String,
    pub schema_version: u32,
    pub description: String,
    pub example: serde_json::Value,
    pub safety: String,
}
```

Add `edit_capabilities: Vec<EditCapability>` to `FormatDescriptor`. XLSX initially
advertises the already-proven cell operations; each task below adds its operation
only after its acceptance tests pass.

- [ ] **Step 2: Inventory potentially affected package parts**

Create:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImpactInventory {
    pub worksheets: Vec<String>,
    pub workbook: bool,
    pub workbook_relationships: bool,
    pub content_types: bool,
    pub shared_strings: bool,
    pub calculation_chain: bool,
    pub tables: Vec<String>,
    pub charts: Vec<String>,
    pub drawings: Vec<String>,
    pub unsupported: Vec<UnsupportedImpact>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedImpact {
    pub part: String,
    pub construct: String,
    pub reason: String,
}
```

Implement `inventory(package, operations)` by reading relationships and XML tags,
not by filename guesses. Sort and deduplicate every result.

- [ ] **Step 3: Add rejection tests**

Create a fixture with a pivot table whose source intersects an inserted row. Assert
validation fails before journaling with:

```text
insert_row is unsafe: unsupported pivot source reference in
xl/pivotCache/pivotCacheDefinition1.xml
```

Create a fixture with an unrelated pivot on another sheet and assert a row edit on
the first sheet may proceed without modifying the pivot part.

- [ ] **Step 4: Verify**

```bash
cargo test -p dotall-xlsx impact
```

Expected: affected unsupported parts reject; unaffected parts do not block.

### Task 2: Implement atomic `set_range`

**Files:**

- Modify: `crates/dotall-xlsx/src/edits/ops.rs`
- Modify: `crates/dotall-xlsx/src/edits/validate.rs`
- Modify: `crates/dotall-xlsx/src/edits/writer/worksheet.rs`
- Create: `crates/dotall-xlsx/tests/set_range.rs`

- [ ] **Step 1: Define rectangular range operation**

Add:

```rust
SetRange {
    sheet: String,
    start_cell: String,
    values: Vec<Vec<EditableCell>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum EditableCell {
    Value(EditableValue),
    Formula(String),
}
```

- [ ] **Step 2: Validate shape and limits**

Validation must:

- reject empty rows and ragged matrices;
- reject overflow beyond XLSX limits (1,048,576 rows; 16,384 columns);
- reject duplicate targets across all operations in the transaction;
- normalize formulas;
- produce one semantic change per target cell in row-major order;
- enforce a configurable maximum cell count before allocating the expanded edit
  list; default 10,000 cells.

- [ ] **Step 3: Patch each affected worksheet once**

Expand the rectangle to sorted typed cell edits, merge with other cell edits, and
run one worksheet event pass. Do not invoke the ZIP writer once per cell.

- [ ] **Step 4: Test transaction atomicity**

Test a 2×3 mixed value/formula range, invalid ragged input, out-of-bounds input,
duplicate target, and a failure on the last cell. The final case must leave all six
cells unchanged.

Run:

```bash
cargo test -p dotall-xlsx --test set_range
```

Expected: all tests pass; advertise `set_range`.

### Task 3: Build reusable coordinate transformations

**Files:**

- Create: `crates/dotall-xlsx/src/edits/transform/address.rs`
- Create: `crates/dotall-xlsx/src/edits/transform/formula.rs`
- Create: `crates/dotall-xlsx/src/edits/transform/sqref.rs`
- Create: `crates/dotall-xlsx/src/edits/transform/mod.rs`

- [ ] **Step 1: Define axis transformations**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Row,
    Column,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisChange {
    Insert { axis: Axis, at: u32, count: u32 },
    Delete { axis: Axis, at: u32, count: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransformResult<T> {
    Kept(T),
    Removed,
    RefError,
}
```

Use one-based row/column indices.

- [ ] **Step 2: Transform cells and ranges**

Add table-driven tests covering:

- coordinates before, within, and after insertion/deletion;
- absolute and relative A1 markers;
- ranges spanning the boundary;
- fully deleted ranges;
- Excel row/column limits;
- deterministic `#REF!` results.

Implement pure functions:

```rust
pub fn transform_cell(cell: &CellRef, change: AxisChange) -> TransformResult<CellRef>;
pub fn transform_range(range: &RangeRef, change: AxisChange) -> TransformResult<RangeRef>;
```

- [ ] **Step 3: Rewrite formula references without evaluating**

Extend the formula lexer to retain byte spans. Rewrite only recognized reference
spans, preserving every other byte exactly. Respect sheet qualification: an
unqualified reference changes only when the formula cell is on the edited sheet;
qualified references change when their target sheet matches.

Tests must cover strings, escaped quotes, ranges, quoted sheet names, mixed absolute
references, named ranges, and `#REF!`.

- [ ] **Step 4: Rewrite whitespace-separated `sqref` lists**

Implement validation and transformation for XML attributes used by data
validations, conditional formatting, ignored errors, and protected ranges.

Run:

```bash
cargo test -p dotall-xlsx edits::transform
```

Expected: all pure transformation tests pass before package mutation begins.

### Task 4: Insert and delete rows

**Files:**

- Modify: `crates/dotall-xlsx/src/edits/ops.rs`
- Modify: `crates/dotall-xlsx/src/edits/validate.rs`
- Modify: `crates/dotall-xlsx/src/edits/writer/worksheet.rs`
- Modify: `crates/dotall-xlsx/src/edits/writer/workbook.rs`
- Create: `crates/dotall-xlsx/tests/structural_rows_columns.rs`

- [ ] **Step 1: Add typed row operations**

```rust
InsertRow { sheet: String, at: u32, count: u32 }
DeleteRow { sheet: String, at: u32, count: u32 }
```

Reject zero count, out-of-range operations, and any operation whose impact inventory
contains unsupported affected constructs.

- [ ] **Step 2: Patch worksheet constructs**

Transform:

- row `r` and cell `r` addresses;
- worksheet dimension;
- merged-cell ranges;
- formula references on every worksheet;
- hyperlinks;
- auto-filter ranges;
- table ranges and table-column calculated formulas;
- data validation and conditional-format `sqref`;
- print titles/areas and workbook defined names;
- chart-series formulas discovered by impact inventory.

Delete rows/cells inside the removed interval. Preserve styles on moved cells.
Remove calc chain after structural changes.

- [ ] **Step 3: Add cross-part golden tests**

Use fixtures combining formulas, named ranges, tables, filters, validations, merges,
charts, and styles. Assert:

- every affected reference shifts correctly;
- removed references become `#REF!` only where Excel semantics require it;
- unrelated parts remain byte-identical;
- output opens through `calamine`;
- revert restores exact original bytes.

Run:

```bash
cargo test -p dotall-xlsx --test structural_rows_columns row
```

Expected: insert/delete row tests pass; advertise both operations.

### Task 5: Insert and delete columns

**Files:**

- Modify the same operation, validation, worksheet, workbook, and golden-test files
  as Task 4.

- [ ] **Step 1: Add typed column operations**

```rust
InsertColumn { sheet: String, at: u32, count: u32 }
DeleteColumn { sheet: String, at: u32, count: u32 }
```

- [ ] **Step 2: Reuse axis-generic transforms**

No separate column-shifting algorithm is permitted. Route through `Axis::Column`
and the same impact/rewrite pipeline used by rows.

- [ ] **Step 3: Test Excel's XFD boundary**

Test insertion at A, inside populated data, and at XFD; deletion spanning formulas,
tables, charts, merged cells, and defined names. Reject overflow past XFD.

Run:

```bash
cargo test -p dotall-xlsx --test structural_rows_columns column
```

Expected: insert/delete column tests pass; advertise both operations.

### Task 6: Add and rename sheets

**Files:**

- Modify: `crates/dotall-xlsx/src/edits/ops.rs`
- Modify: `crates/dotall-xlsx/src/edits/validate.rs`
- Modify: `crates/dotall-xlsx/src/edits/writer/package.rs`
- Create: `crates/dotall-xlsx/src/edits/writer/workbook.rs`
- Create: `crates/dotall-xlsx/tests/structural_sheets.rs`

- [ ] **Step 1: Add operations and name validation**

```rust
AddSheet { name: String, after: Option<String> }
RenameSheet { from: String, to: String }
```

Enforce Excel sheet-name rules: 1–31 characters, no `[]:*?/\\`, no leading/trailing
apostrophe, and case-insensitive uniqueness.

- [ ] **Step 2: Add sheets surgically**

Allocate the lowest unused relationship ID, sheet ID, and worksheet part number.
Patch:

- `xl/workbook.xml`;
- `xl/_rels/workbook.xml.rels`;
- `[Content_Types].xml`;
- `docProps/app.xml` sheet-title vector when present;
- the new minimal worksheet part.

Raw-copy every other part.

- [ ] **Step 3: Rename all semantic references**

Rename the workbook sheet entry and rewrite sheet-qualified references in worksheet
formulas, defined names, charts, tables, validations, and print settings. Quote the
new name when Excel syntax requires it.

- [ ] **Step 4: Verify golden cases**

Test simple names, spaces, apostrophes, case-only collisions, formulas, charts,
defined names, and revert.

Run:

```bash
cargo test -p dotall-xlsx --test structural_sheets add
cargo test -p dotall-xlsx --test structural_sheets rename
```

Expected: add/rename tests pass; advertise both operations.

### Task 7: Delete sheets with explicit dependency policy

**Files:**

- Modify the sheet operation, validation, package writer, and sheet tests.

- [ ] **Step 1: Add operation policy**

```rust
DeleteSheet {
    name: String,
    dependency_policy: DeleteSheetPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeleteSheetPolicy {
    RejectIfReferenced,
    ReplaceReferencesWithRefError,
}
```

Default agent-facing examples use `reject_if_referenced`.

- [ ] **Step 2: Validate workbook invariants**

Reject deletion of the final visible sheet. Inventory all inbound references and
return them in the validation error under reject policy.

- [ ] **Step 3: Patch package membership**

Remove the sheet entry, relationship, worksheet part, worksheet relationships, and
content-type override. Remove or rewrite defined names and references according to
policy. Remove orphaned drawings/charts only when no remaining relationship points
to them.

- [ ] **Step 4: Test both policies and orphan handling**

Run:

```bash
cargo test -p dotall-xlsx --test structural_sheets delete
```

Expected: referenced deletion rejects by default; explicit replacement produces
valid `#REF!`; unrelated shared parts remain.

### Task 8: CLI, capability, and quality gate

**Files:**

- Modify: `crates/dotall-cli/src/main.rs`
- Modify: CLI edit tests
- Add approved fixtures under: `fixtures/xlsx/structural/`

- [ ] **Step 1: Use the existing `--op` envelope**

No new top-level CLI command is needed. Ensure `inspect` advertises every operation
only after its tests pass and returns exact JSON examples.

- [ ] **Step 2: Add one end-to-end test per operation**

For each operation: inspect capability, submit edit with expected hash, validate
semantic diff, reopen output, inspect history, and revert to exact original.

- [ ] **Step 3: Run full verification**

```bash
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
git diff --check
```

Expected: all checks pass.

- [ ] **Step 4: Optional commit checkpoints**

Create one explicitly authorized commit per independently passing operation family:

```text
feat(xlsx): set rectangular ranges
feat(xlsx): insert and delete rows
feat(xlsx): insert and delete columns
feat(xlsx): add and rename sheets
feat(xlsx): safely delete sheets
```

## Self-review record

- Covers every broader v0 operation from the XLSX spec.
- Makes impact analysis and rejection part of correctness rather than claiming
  unsafe universal support.
- Reuses core transactions and axis-generic transforms instead of duplicating
  lifecycle or row/column algorithms.
