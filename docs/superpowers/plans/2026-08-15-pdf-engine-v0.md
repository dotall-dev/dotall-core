# PDF Engine v0 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `dotall-pdf` so agents can inspect pages and AcroForm fields, fill a text field, and revert `.pdf` files. No designed-page body editing.

**Architecture:** New format crate. `lopdf` loads the document, lists pages/outline/fields, and writes field `/V` updates. Snapshots use the **default opaque** `FormatHandler` blob (not OOXML parts). Register behind `--features pdf`.

**Tech Stack:** `lopdf` (pin a current 0.3x/0.4x that builds on edition 2024 — verify at implementation time and record the exact version in `Cargo.toml`), blake3, serde JSON.

**Prerequisite:** [`2026-08-15-docx-engine-v0.md`](2026-08-15-docx-engine-v0.md) — **done on `feat/docx-engine-v0`.** CLI already composes xlsx/pptx/docx and has `--selector-kind`.

**Design:** [`docs/superpowers/specs/2026-08-15-office-pdf-format-families-design.md`](../specs/2026-08-15-office-pdf-format-families-design.md)

## Grounded APIs (after DOCX)

- Do **not** use `dotall-ooxml`. PDF snapshots stay on the default opaque `FormatHandler::encode_snapshot` / `decode_snapshot`.
- Register like other formats:

```rust
#[cfg(feature = "pdf")]
registry.register(Arc::new(dotall_pdf::PdfFormat));
```

- CLI `--selector-kind page|field|full` is already wired.
- Pin `lopdf = "0.44.0"` (edition 2024). If APIs differ, adapt; do not add a second PDF crate.
- Copy `PptxFormat`/`DocxFormat` envelope + inspect/read/validate/apply shape; omit snapshot overrides.

**Branch:** `feat/pdf-engine-v0` (stacked on `feat/docx-engine-v0`).

## Global Constraints

- Format id `pdf`; schema `pdf.document` v1
- Edits: **`set_form_field` only**
- Reject encrypted (`trailer Encrypt`) and certified/signed docs (presence of `/ByteRange` + `/Contents` on a signature field, or `/Perms` in catalog)
- Text extraction is best-effort; tests use a fixture whose page text is known
- Do not rewrite the whole PDF via a print pipeline
- No `unwrap` in library code
- Pin `lopdf` exactly; if API differs from snippets below, adapt the snippets to the pinned crate — do not add a second PDF library

---

## File structure

- Create: `crates/dotall-pdf/Cargo.toml`
- Create: `crates/dotall-pdf/src/{lib,format,detection,model,ids,parser,projection,selector,edits}.rs`
- Create: `crates/dotall-pdf/tests/{read_contract,form_edit}.rs`
- Create: `crates/dotall-pdf/tests/fixtures/` **or** generate PDFs in tests with `lopdf` (preferred: generate, no binary in git)
- Create: `skills/pdf/SKILL.md`
- Modify: workspace + CLI/MCP features and registries
- Modify: `AGENTS.md`

---

### Task 1: Crate, detection, model

**Interfaces:**
- `FORMAT_ID = "pdf"`
- Detection: prefix `b"%PDF-"` → 100 if path ends with `.pdf` or prefix matches (`.pdf` + `%PDF-` → 100; `%PDF-` without extension → 80; `.pdf` without magic → 40; else 0)
- Model:

```rust
pub const SCHEMA_ID: &str = "pdf.document";
pub const SCHEMA_VERSION: u32 = 1;

pub struct PdfDocumentModel {
    pub document_id: String,
    pub page_count: u32,
    pub pages: Vec<PdfPageModel>,
    pub fields: Vec<PdfFieldModel>,
    pub outline: Vec<String>,
    pub encrypted: bool,
}

pub struct PdfPageModel {
    pub element_id: String,
    pub number: u32, // 1-based
    pub text: String,
}

pub struct PdfFieldModel {
    pub element_id: String,
    pub name: String,      // fully qualified /T path if possible
    pub field_type: String, // "tx" | "btn" | "ch" | "sig" | "unknown"
    pub value: String,
    pub page: Option<u32>,
    pub read_only: bool,
}
```

Ids: `pdf_`, `pg_`, `fl_` domain `pdf.document`.

- [ ] **Step 1: Tests** — `%PDF-` + `.pdf` scores 100; `PK\x03\x04` scores 0.

- [ ] **Step 2: Implement crate + detection**

- [ ] **Step 3: Commit** `feat(pdf): add format crate and detection`

---

### Task 2: Parse pages, outline, fields

**Interfaces:**
- `parse_pdf_bytes(&[u8]) -> Result<PdfDocumentModel>`

Using pinned `lopdf`:

1. `Document::load_mem(bytes)`
2. If trailer has `Encrypt` → `encrypted: true` and **still return** a model with empty pages/fields (inspect can warn). `validate_edit` will reject.
3. Page count from `doc.get_pages()`
4. Text: concatenate string objects from each page’s content stream in order (simple `Tj`/`TJ` / literal strings). If extraction fails, `text` is empty string (not an error).
5. Fields: walk `AcroForm` `/Fields` recursively; read `/T`, `/FT`, `/V`, `/Ff` bit 1 for read-only; `/Sig` type recorded as `sig`
6. Outline: `/Outlines` title strings if present

- [ ] **Step 1: Write `minimal_form_pdf()` helper** that uses `lopdf` to create a 1-page PDF with a text field `Name` value `Ada` and page content showing `Hello`.

If creating a valid AcroForm in `lopdf` is too thick, check in **one** tiny generated fixture under `crates/dotall-pdf/tests/fixtures/name-field.pdf` produced once by a `#[ignore]` generator test — prefer generating in `minimal_form_pdf()` so CI needs no binary.

- [ ] **Step 2: Test**

```rust
#[test]
fn parses_form_field_and_page_count() {
    let model = parse_pdf_bytes(&minimal_form_pdf()).unwrap();
    assert_eq!(model.page_count, 1);
    assert_eq!(model.fields[0].name, "Name");
    assert_eq!(model.fields[0].value, "Ada");
    assert!(!model.encrypted);
}
```

- [ ] **Step 3: Implement parser + `parse`/`parse_bytes`**

- [ ] **Step 4: Commit** `feat(pdf): parse pages and AcroForm fields`

---

### Task 3: Inspect and reads

**Interfaces:**
- Selectors: `full`; `page` (`1`); `field` (`Name`)
- Inspect summary:

```json
{
  "page_count": 1,
  "field_names": ["Name"],
  "encrypted": false,
  "has_signature": false
}
```

`has_signature`: any field `field_type == "sig"`.

Projection: markdown pages + `## Fields` table-free list `` `Name` (tx) = Ada ``.

- [ ] **Step 1: `read_contract.rs` tests** for `page` and `field` selectors

- [ ] **Step 2: Implement inspect/read**

- [ ] **Step 3: Commit** `feat(pdf): inspect and page/field reads`

---

### Task 4: `set_form_field`

**Interfaces:**

```json
{ "kind": "set_form_field", "payload": { "name": "Name", "value": "Grace" } }
```

Validation:

- `encrypted` → error `cannot edit encrypted PDF`
- `has_signature` / `field_type == "sig"` on any field → error `cannot edit signed PDF`
- Missing name → error
- `read_only` → error `field is read-only`
- v0 supports `tx` (and `ch` if `/V` is a string). `btn` (checkbox/radio) **reject** with `checkbox/radio not supported in v0`

Apply (`apply_edit_bytes`):

1. `Document::load_mem`
2. Find the field dictionary by `/T` (recursive)
3. Set `/V` to a `Object::string_literal` of the new value
4. Set `/NeedAppearances` true on AcroForm so viewers regenerate appearance
5. `doc.save_to(&mut Vec<u8>)`
6. `PatchedOutput { after_source_hash: blake3 hex, bytes }`

- [ ] **Step 1: Tests**
  - Fill Name → parse value Grace
  - Encrypted fixture (or set Encrypt dict in helper) rejected at validate
  - Unknown field rejected

- [ ] **Step 2: Implement + capability**

- [ ] **Step 3: Commit** `feat(pdf): fill AcroForm text fields`

---

### Task 5: Opaque snapshots via Engine

PDF uses default `encode_snapshot` (full blob). Add a test that Engine apply stores a snapshot and revert restores original bytes.

**Files:** `crates/dotall-pdf/tests/form_edit.rs` (Engine + temp `.all/`)

- [ ] **Step 1: Test revert after `set_form_field` restores original field value and file hash**

- [ ] **Step 2: Do not override encode/decode** unless default opaque path fails the test — then fix core, not a custom PDF splitter

- [ ] **Step 3: Commit** `test(pdf): history revert after form fill`

---

### Task 6: CLI, MCP, skill

- Feature `pdf` on CLI/MCP; add to `default` on ship commit
- Register `PdfFormat`
- `skills/pdf/SKILL.md` — emphasize form fill only; never ask the agent to rewrite page content streams
- CLI: `--selector-kind page --selector 1`
- `cargo test --workspace`, clippy `-D warnings`, fmt

- [ ] **Step 1: `crates/dotall-cli/tests/pdf_read.rs`**

- [ ] **Step 2: AGENTS.md** crate list + non-goal reminder (no PDF body edit)

- [ ] **Step 3: Commit** `feat(pdf): register CLI/MCP and add agent skill`

---

## Spec coverage

- PDF v0 read + form fill + opaque snapshots: Tasks 1–6
- Annotations, page rotate/delete, incremental xref, body text edit: out of v0
