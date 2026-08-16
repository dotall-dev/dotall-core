# PPTX Engine v0 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `dotall-pptx` so agents can inspect, read, surgically set shape text, and revert `.pptx` files through the existing Engine/MCP without unzipping.

**Architecture:** New format crate behind `FormatHandler`. Parse `ppt/presentation.xml` + each `ppt/slides/slideN.xml` with `quick-xml`. Patch only the target slide part. Snapshots via `dotall-ooxml::encode_package`. CLI/MCP register `PptxFormat` behind `--features pptx`.

**Tech Stack:** `zip`, `quick-xml`, `dotall-ooxml`, blake3, serde JSON.

**Prerequisite:** [`2026-08-15-ooxml-shared-snapshots.md`](2026-08-15-ooxml-shared-snapshots.md)

**Design:** [`docs/superpowers/specs/2026-08-15-office-pdf-format-families-design.md`](../specs/2026-08-15-office-pdf-format-families-design.md)

## Global Constraints

- Format id `pptx`; schema `pptx.presentation` v1
- Untouched ZIP entries stay byte-identical after `set_shape_text`
- Reject SmartArt, charts, and grouped drawingML the writer cannot patch
- No slide add/delete/reorder in v0
- Manifest schema id `pptx.snapshot-manifest`
- Default CLI/MCP features: keep `xlsx`; add `pptx` to `default` only after tests pass (include in default for the ship commit)
- `Result` everywhere; no `unwrap` in library code

---

## File structure

- Create: `crates/dotall-pptx/Cargo.toml`
- Create: `crates/dotall-pptx/src/{lib,format,detection,model,ids,parser,projection,selector,edits}.rs`
- Create: `crates/dotall-pptx/tests/{read_contract,surgical_edit,snapshot_parts}.rs`
- Create: `fixtures/pptx/minimal.pptx` (generated in tests via zip+xml, not a binary checked in if avoidable)
- Create: `skills/pptx/SKILL.md`
- Modify: workspace `Cargo.toml`, `crates/dotall-cli/Cargo.toml`, `crates/dotall-mcp/Cargo.toml`
- Modify: `crates/dotall-cli/src/main.rs` (`default_registry`, generic read flags)
- Modify: `crates/dotall-mcp/src/server.rs` (`registry()`)
- Modify: `AGENTS.md`

---

### Task 1: Crate skeleton, detection, model, ids

**Files:**
- Create: `crates/dotall-pptx/**`
- Modify: workspace `Cargo.toml`

**Interfaces:**
- Produces: `pub const FORMAT_ID: &str = "pptx"`
- Produces: `PptxFormat` (empty `FormatHandler` stubs that compile)
- Produces: `detection::score` — `.pptx` + ZIP magic → 100; `.pptx` without magic → 40; else 0
- Produces: model types below

```rust
pub const SCHEMA_ID: &str = "pptx.presentation";
pub const SCHEMA_VERSION: u32 = 1;

pub struct PresentationModel {
    pub presentation_id: String,
    pub slides: Vec<SlideModel>,
}

pub struct SlideModel {
    pub element_id: String,
    pub name: String,      // "Slide 1"
    pub index: u32,        // 0-based
    pub part_name: String, // "ppt/slides/slide1.xml"
    pub shapes: Vec<ShapeModel>,
    pub notes: Option<String>,
}

pub struct ShapeModel {
    pub element_id: String,
    pub name: String, // nvSpPr cNvPr name, or "shape:{idx}"
    pub text: String, // concatenated a:t
}
```

`ids`: opaque blake3 prefixes `pr_`, `sl_`, `sp_` hashed with schema id `pptx.presentation` (copy `dotall-xlsx/src/ids.rs` pattern, change domain string).

- [ ] **Step 1: Write failing tests in `crates/dotall-pptx/src/lib.rs`**

```rust
#[test]
fn format_id_is_stable() {
    assert_eq!(FORMAT_ID, "pptx");
}

#[test]
fn pptx_extension_plus_zip_magic_scores_100() {
    let score = detection::score(&DetectionProbe {
        path: Path::new("deck.pptx"),
        prefix: b"PK\x03\x04",
    });
    assert_eq!(score.0, 100);
}

#[test]
fn xlsx_path_does_not_match_pptx_handler() {
    let score = detection::score(&DetectionProbe {
        path: Path::new("book.xlsx"),
        prefix: b"PK\x03\x04",
    });
    assert_eq!(score.0, 0);
}
```

- [ ] **Step 2: Run** `cargo test -p dotall-pptx --lib`  
  Expected: FAIL (crate missing).

- [ ] **Step 3: Implement crate + detection + ids + model structs**  
  Stub `FormatHandler` methods that return `UnsupportedCapability` except `detect` and `descriptor`.

- [ ] **Step 4: Run tests** — Expected: PASS.

- [ ] **Step 5: Commit** `feat(pptx): add format crate and detection`

---

### Task 2: Parse presentation + slides

**Files:**
- Create: `crates/dotall-pptx/src/parser.rs`
- Test: `crates/dotall-pptx/tests/read_contract.rs`

**Interfaces:**
- Consumes: ZIP bytes
- Produces: `parse_presentation_bytes(&[u8]) -> Result<PresentationModel>`

Parser algorithm:

1. Open `ZipArchive` on a `Cursor`.
2. Read `ppt/presentation.xml`; collect `p:sldIdLst/p:sldId` relationship ids in order.
3. Read `ppt/_rels/presentation.xml.rels`; map `Id` → `Target` (resolve relative to `ppt/`).
4. For each slide part, parse `p:sp` (and `p:txBody`) with `quick-xml`. Concatenate descendant `a:t` text. Shape name from `p:nvSpPr/p:cNvPr/@name` or `shape:{i}`.
5. Skip `p:graphicFrame` (charts/SmartArt) — do not emit shapes for them.
6. Notes: if `ppt/notesSlides/notesSlideN.xml` exists, concatenate `a:t`; else `None`.

- [ ] **Step 1: Write a fixture builder in the test file**

Build a minimal PPTX in memory (ZIP of `[Content_Types].xml`, `_rels/.rels`, `ppt/presentation.xml`, rels, one `ppt/slides/slide1.xml` with a shape named `Title` and text `Hello`). Do not depend on PowerPoint.

- [ ] **Step 2: Assert parse finds one slide, one shape, text `Hello`**

```rust
#[test]
fn parses_title_shape_text() {
    let bytes = minimal_pptx();
    let model = parse_presentation_bytes(&bytes).expect("parse");
    assert_eq!(model.slides.len(), 1);
    assert_eq!(model.slides[0].shapes[0].name, "Title");
    assert_eq!(model.slides[0].shapes[0].text, "Hello");
}
```

- [ ] **Step 3: Run test** — Expected: FAIL (`parse_presentation_bytes` missing).

- [ ] **Step 4: Implement parser**

- [ ] **Step 5: Wire `PptxFormat::parse` / `parse_bytes`** to serialize `PresentationModel` into `ArtifactEnvelope.payload`.

- [ ] **Step 6: Run tests** — Expected: PASS.

- [ ] **Step 7: Commit** `feat(pptx): parse slides and shape text`

---

### Task 3: Inspect, selectors, projections

**Files:**
- Create: `crates/dotall-pptx/src/selector.rs`
- Create: `crates/dotall-pptx/src/projection.rs`
- Modify: `crates/dotall-pptx/src/format.rs`
- Test: `crates/dotall-pptx/tests/read_contract.rs`

**Interfaces:**
- Selectors: `full` (value ignored), `slide` (`Slide 1` or `slide1` or `1`), `notes` (same slide identity)
- `inspect.summary` JSON:

```json
{
  "slides": [{ "name": "Slide 1", "index": 0, "shape_count": 1, "preview": "Hello" }],
  "media_parts": ["ppt/media/image1.png"]
}
```

`media_parts`: ZIP names under `ppt/media/` (inspect only).

Projections: markdown `# Slide 1` then `- **Title**: Hello`. Honor `max_tokens` by truncating shapes and setting `truncated` + numeric `continuation` like XLSX.

- [ ] **Step 1: Tests** — inspect suggested_reads includes `slide`; `read` with `selector_kind=slide` returns Hello; second read with continuation if forced `max_tokens=1`.

- [ ] **Step 2: Implement inspect/read**

- [ ] **Step 3: `cargo test -p dotall-pptx --test read_contract`** — PASS

- [ ] **Step 4: Commit** `feat(pptx): inspect and token-budgeted slide reads`

---

### Task 4: `set_shape_text` surgical apply

**Files:**
- Create: `crates/dotall-pptx/src/edits.rs`
- Test: `crates/dotall-pptx/tests/surgical_edit.rs`

**Interfaces:**
- Operation:

```json
{ "kind": "set_shape_text", "payload": { "slide": "Slide 1", "shape": "Title", "text": "World" } }
```

Validation:

- Resolve slide by `SlideModel.name` or `part_name` stem
- Resolve shape by `name` or `element_id`
- If the shape’s XML subtree contains `a:graphic` / `p:graphicFrame` / `mc:AlternateContent` that is not a simple `p:sp`+`p:txBody`, return format error `cannot edit non-text shape`
- Expand to a single `SemanticOperation` in `ValidatedEdit` with `semantic_diff` target `Slide 1!Title`

Apply:

1. Load slide XML bytes from the ZIP (`slide.part_name`)
2. Find the `p:sp` whose `cNvPr/@name` matches
3. Replace **all** `a:t` text in that shape’s `p:txBody` with a single run: keep the first `a:r`/`a:rPr`, set its `a:t` to the new string, delete extra `a:r` siblings inside `a:p` (v0: one paragraph). If `txBody` has multiple `a:p`, set first para only and clear remaining `a:t` to empty (document this in the capability example).
4. Rebuild the package with the same ZIP raw-copy strategy as XLSX `rebuild_package` — **copy this function into `dotall-pptx` for v0** (do not extract a shared writer yet). Replace only `ppt/slides/slideN.xml`.

- [ ] **Step 1: Test** — after apply, parse text is `World`; compare zip entries: every name except the target slide XML has identical compressed bytes (iterate both archives).

```rust
#[test]
fn set_shape_text_leaves_other_parts_byte_identical() {
    let before = minimal_pptx();
    // ... validate + apply_edit_bytes ...
    assert_untouched_entries_identical(&before, &patched.bytes, &["ppt/slides/slide1.xml"]);
}
```

- [ ] **Step 2: Run** — FAIL (no edits module).

- [ ] **Step 3: Implement validate + patch**

- [ ] **Step 4: Capability** — `edit_capabilities` one entry `set_shape_text` with the JSON example above and safety note “text frames only”.

- [ ] **Step 5: Tests PASS**

- [ ] **Step 6: Commit** `feat(pptx): surgically set shape text`

---

### Task 5: Snapshots + Engine round-trip

**Files:**
- Test: `crates/dotall-pptx/tests/snapshot_parts.rs`
- Modify: `format.rs` encode/decode

**Interfaces:**
- `encode_snapshot_with_hash` → `dotall_ooxml::encode_package(..., "pptx", "pptx.snapshot-manifest", Some(hash))`

- [ ] **Step 1: Test** — Engine `init` temp dir, copy fixture, `set_shape_text`, `apply`; `decode_snapshot` of history snapshot equals pre-apply bytes; `parts/` after two text edits on the same slide is ≪ 2× file size (media/theme parts shared).

- [ ] **Step 2: Implement encode/decode on `PptxFormat`**

- [ ] **Step 3: PASS + commit** `feat(pptx): reuse OOXML part snapshots`

---

### Task 6: CLI, MCP, skill

**Files:**
- Modify: `crates/dotall-cli/Cargo.toml` — `pptx = ["dep:dotall-pptx"]`, `default = ["xlsx", "pptx"]`
- Modify: `crates/dotall-mcp/Cargo.toml` — same
- Modify: `default_registry` / MCP `registry()`:

```rust
#[cfg(feature = "xlsx")]
registry.register(Arc::new(dotall_xlsx::XlsxFormat));
#[cfg(feature = "pptx")]
registry.register(Arc::new(dotall_pptx::PptxFormat));
```

Remove `#[cfg(not(feature = "xlsx"))]` exclusive empty registry; compose independently.

CLI `Read`: add `--selector-kind` and `--selector` (conflict with xlsx-only `--sheet`/`--range`/`--ast_range`). When set, build `ReadSelector { kind, value }` without requiring xlsx.

CLI `Edit`: already has `--op` + `--payload-json`; works for pptx if `require_xlsx_support` is **not** gating all edits. Replace `require_xlsx_support` with “registry must detect the file.”

- [ ] **Step 1: Integration test** `crates/dotall-cli/tests/pptx_read.rs` using `assert_cmd` inspect on a temp pptx.

- [ ] **Step 2: `skills/pptx/SKILL.md`** — copy xlsx skill structure; ops from capabilities; no deps tool required for v0.

- [ ] **Step 3: AGENTS.md** crate list + `skills/pptx/SKILL.md`

- [ ] **Step 4:** `cargo test --workspace` && `cargo clippy --workspace -- -D warnings` && `cargo fmt`

- [ ] **Step 5: Commit** `feat(pptx): register CLI/MCP and add agent skill`

---

## Spec coverage

- PPTX model/read/edit/snapshots/skill: Tasks 1–6
- Slide add/delete: explicitly out of v0
- Shared writer extract: not in this plan
