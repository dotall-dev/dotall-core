# DOCX Engine v0 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `dotall-docx` so agents can inspect, read paragraphs, surgically set paragraph text, and revert `.docx` files through Engine/MCP.

**Architecture:** Parse `word/document.xml` body `w:p` nodes. `set_paragraph_text` patches that `w:p` only and rebuilds the ZIP with raw-copy of every other entry. Snapshots via `dotall-ooxml`. Register behind `--features docx`.

**Tech Stack:** `zip`, `quick-xml`, `dotall-ooxml`, blake3, serde JSON.

**Prerequisite:** [`2026-08-15-pptx-engine-v0.md`](2026-08-15-pptx-engine-v0.md) — **done on `feat/pptx-engine-v0`.**

**Design:** [`docs/superpowers/specs/2026-08-15-office-pdf-format-families-design.md`](../specs/2026-08-15-office-pdf-format-families-design.md)

## Grounded APIs (after PPTX)

Use these; do not re-extract snapshot or ZIP-probe code:

```rust
dotall_ooxml::encode_package(bytes, format_id, manifest_schema_id, package_hash: Option<&str>)
    -> Result<EncodedSnapshot>
dotall_ooxml::decode_package(&encoded) -> Result<Vec<u8>>
dotall_ooxml::has_zip_magic(prefix: &[u8]) -> bool
```

Copy `rebuild_package` from `crates/dotall-pptx/src/edits.rs` (raw-copy untouched ZIP entries). Do not extract a shared writer yet.

CLI already has `--selector-kind` / `--selector`. DOCX reads use `paragraphs` + `0:2` or `0`. Compose the registry independently:

```rust
#[cfg(feature = "xlsx")]
registry.register(Arc::new(dotall_xlsx::XlsxFormat));
#[cfg(feature = "pptx")]
registry.register(Arc::new(dotall_pptx::PptxFormat));
#[cfg(feature = "docx")]
registry.register(Arc::new(dotall_docx::DocxFormat));
```

`PptxFormat` is the template for `DocxFormat` (`parse` → JSON envelope, `inspect`/`read`/`validate_edit`/`apply_edit`, `encode_package(..., "docx", "docx.snapshot-manifest", None)`).

**Branch:** `feat/docx-engine-v0` (stacked on `feat/pptx-engine-v0`).

## Global Constraints

- Format id `docx`; schema `docx.document` v1
- Untouched ZIP entries byte-identical after body paragraph edits (`word/document.xml` is the allowed change)
- Reject `set_paragraph_text` when the target `w:p` contains `w:del`/`w:ins` (tracked changes), `w:sdt`, or `w:fldChar`/`w:instrText`
- No header/footer/comment/style edits in v0
- Manifest schema id `docx.snapshot-manifest`
- Intra-`document.xml` deltas are **out of v0**
- No `unwrap` in library code

---

## File structure

- Create: `crates/dotall-docx/src/{lib,format,detection,model,ids,parser,projection,selector,edits}.rs`
- Create: `crates/dotall-docx/tests/{read_contract,surgical_edit,snapshot_parts}.rs`
- Create: `skills/docx/SKILL.md`
- Modify: workspace + CLI/MCP Cargo.toml and registries
- Modify: `AGENTS.md`

---

### Task 1: Crate, detection, model, ids

**Interfaces:**
- `FORMAT_ID = "docx"`
- Detection: `.docx` + ZIP magic → 100; `.docx` without magic → 40; else 0
- Model:

```rust
pub const SCHEMA_ID: &str = "docx.document";
pub const SCHEMA_VERSION: u32 = 1;

pub struct DocumentModel {
    pub document_id: String,
    pub paragraphs: Vec<ParagraphModel>,
}

pub struct ParagraphModel {
    pub element_id: String,
    pub index: u32,              // 0-based body order
    pub outline_level: Option<u32>, // w:outlineLvl val, or heading style 1–9 if present as style name Heading N
    pub style_id: Option<String>,   // w:pStyle w:val
    pub text: String,               // concatenated w:t
}
```

Ids: prefixes `doc_`, `p_` with domain `docx.document`.

- [ ] **Step 1: Failing tests** — `FORMAT_ID`, `.docx` scores 100, `.pptx` scores 0 on this handler.

- [ ] **Step 2: Implement crate + stubs**

- [ ] **Step 3: PASS + commit** `feat(docx): add format crate and detection`

---

### Task 2: Parse `word/document.xml`

**Interfaces:**
- `parse_document_bytes(&[u8]) -> Result<DocumentModel>`

Algorithm:

1. ZIP open; require `word/document.xml`.
2. Stream `quick-xml` events. For each `w:p` in `w:body` (not in `w:tbl` for v0 — **skip table paragraphs** entirely so we do not emit ids we cannot safely patch yet).
3. Collect `w:t` text (honor `xml:space="preserve"`).
4. Read `w:pPr/w:pStyle/@w:val` and `w:outlineLvl/@w:val`.

- [ ] **Step 1: Fixture** — minimal DOCX ZIP (`[Content_Types].xml`, rels, `word/document.xml` with two `w:p`: “Alpha”, “Beta”).

- [ ] **Step 2: Test**

```rust
#[test]
fn parses_body_paragraphs_in_order() {
    let model = parse_document_bytes(&minimal_docx()).unwrap();
    assert_eq!(model.paragraphs.len(), 2);
    assert_eq!(model.paragraphs[0].text, "Alpha");
    assert_eq!(model.paragraphs[1].text, "Beta");
}
```

- [ ] **Step 3: Implement parser + `parse`/`parse_bytes`**

- [ ] **Step 4: Commit** `feat(docx): parse body paragraphs`

---

### Task 3: Inspect and reads

**Interfaces:**
- Selectors: `full`; `paragraphs` value `0:2` (inclusive start, exclusive end) or `0` for one para
- Inspect summary:

```json
{
  "paragraph_count": 2,
  "headings": [{ "index": 0, "text": "Alpha", "style_id": "Heading1" }],
  "skipped_tables": true
}
```

`skipped_tables`: true if `w:tbl` present.

Projection: numbered markdown `0. Alpha`. Budget + continuation like XLSX.

- [ ] **Step 1: Tests** in `read_contract.rs`

- [ ] **Step 2: Implement**

- [ ] **Step 3: Commit** `feat(docx): inspect and paragraph reads`

---

### Task 4: `set_paragraph_text`

**Interfaces:**

```json
{ "kind": "set_paragraph_text", "payload": { "index": 1, "text": "Gamma" } }
```

Also accept `element_id` instead of `index`.

Validation:

- Bounds-check index
- Re-open `document.xml`; locate Nth body `w:p` (same skip-tables rule as parse)
- If that element’s inner XML contains `w:del`, `w:ins`, `w:sdt`, `w:fldChar`, or `w:instrText` → `DotallError::Format` with message `paragraph contains tracked changes, a content control, or a field`

Apply:

1. In that `w:p`, keep `w:pPr` untouched
2. Replace `w:r` children with **one** `w:r`: clone `w:rPr` from the first existing run if any; single `w:t xml:space="preserve"` with the new text
3. ZIP rebuild: replace only `word/document.xml`

- [ ] **Step 1: Tests**
  - Happy path: paragraph 1 becomes Gamma; `[Content_Types].xml` and rels unchanged
  - Reject: paragraph wrapped in `w:ins` fails validate

- [ ] **Step 2: Implement edits + capability example**

- [ ] **Step 3: Commit** `feat(docx): surgically set paragraph text`

---

### Task 5: Snapshots

- `encode_snapshot_with_hash` → `dotall_ooxml::encode_package(..., "docx", "docx.snapshot-manifest", Some(hash))`

- [ ] **Step 1: Test** — apply twice; `parts/` size ≪ 2× file (styles/media shared; `document.xml` may appear twice — assert styles part hash reused)

- [ ] **Step 2: Commit** `feat(docx): reuse OOXML part snapshots`

---

### Task 6: CLI, MCP, skill

- Feature `docx` on CLI/MCP; `default` includes `docx` on the ship commit
- Register `DocxFormat` next to xlsx/pptx
- `skills/docx/SKILL.md`
- CLI inspect/read via `--selector-kind paragraphs --selector 0:10`
- `cargo test --workspace`, clippy `-D warnings`, fmt

- [ ] **Step 1: `crates/dotall-cli/tests/docx_read.rs`** inspect fixture

- [ ] **Step 2: AGENTS.md**

- [ ] **Step 3: Commit** `feat(docx): register CLI/MCP and add agent skill`

---

## Spec coverage

- DOCX v0 model/read/edit/snapshots: Tasks 1–6
- Inner XML deltas, comments, headers: out of v0
- Table cell edits: out of v0 (tables skipped)
