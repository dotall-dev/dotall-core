# Office-Parity Atlas Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Independent formats may run as parallel Task subagents; verify each wave before starting the next for that format.

**Goal:** Ship Waves 23–25 of the Office-parity atlas so agents can inspect and insert comments, inspect and insert pictures (PDF stamps as annotations), inspect Office charts read-only, and apply light report styling — without becoming a full Office clone.

**Architecture:** Keep format-owned models (`xlsx.workbook`, `pptx.presentation`, `docx.document`, `pdf.document`). Grow inspect JSON with parallel `comments[]` / `pictures[]` / `charts[]` arrays (PDF omits `charts`). Inserts are additive ZIP parts + relationships (Office) or new annotation objects (PDF). Validate rejects edit/delete/replace of existing comments or pictures and any chart mutate. Surgical OOXML remains the moat: patch only the parts the op owns.

**Tech Stack:** Stable Rust workspace, `FormatHandler`, `zip` + `quick-xml` (Office), `lopdf` (PDF), blake3 snapshots, serde JSON models, `rust_xlsxwriter` 0.96 fixtures (`Note`, `Image`, `Chart`), CLI (`dotall-cli`) + stdio MCP (`dotall-mcp`).

## Global Constraints

- Surgical OOXML: patch only target parts; untouched ZIP entries stay byte-identical. PDF: new annotation objects only; page content streams stay byte-identical on comment/stamp insert.
- Comments: inspect/read + **insert new only**. Reject edit/delete/replace of an existing comment (including “set comment text”).
- Pictures: inspect/read + **insert new only**. Reject mutate/replace/crop/delete of existing media.
- Charts: inspect/read only. **No insert, no mutate.** Reject with `DotallError::Format` (unsupported edit) or `UnsupportedCapability` — same family as today’s unsupported-kind errors.
- Insert mechanics: additive ZIP/PDF (new parts + rels / new annotation objects).
- PDF stamps: annotation objects (`/Subtype /Stamp` + annot-owned `/AP`), **not** content-stream rewrite.
- IR: format-owned models in `dotall-xlsx` / `dotall-pptx` / `dotall-docx` / `dotall-pdf`. No universal Office AST / unified IR. Field names inside comment/picture/chart objects are format-owned (`sheet` vs `slide` vs `paragraph` vs `page`).
- **XLSX comments = Excel Notes (legacy).** Write and parse `xl/commentsN.xml` + `xl/drawings/vmlDrawingN.vml` + worksheet `legacyDrawing` + rels. rust_xlsxwriter 0.96 generates this via `Note::new(...).set_author(...)` + `worksheet.insert_note(row, col, &note)`. Excel 365 shows them as Notes (red triangle). Modern threaded comments (`xl/threadedComments/`, people.xml) are **out of scope** — rust_xlsxwriter will not emit them (company people metadata).
- **PPTX comments = ECMA-376** `ppt/commentAuthors.xml` + `ppt/comments/commentN.xml` (`p:cmLst` / `p:cm`). Not Office 365 modern `p188` comments.
- **DOCX comments =** `word/comments.xml` + `w:commentRangeStart` / `w:commentRangeEnd` / `w:commentReference` on the target paragraph. Clone Wave 21 `word/numbering.xml` create-if-missing (rels + `[Content_Types].xml`).
- **PDF comments =** `/Type /Annot /Subtype /Text` (sticky). **PDF stamps =** `/Type /Annot /Subtype /Stamp` with appearance owned by the annot.
- SCHEMA_VERSION stays `1`. New model fields use `#[serde(default, skip_serializing_if = "Vec::is_empty")]`. Inspect always emits `comments` / `pictures` as arrays (possibly `[]`); Office inspect always emits `charts` (possibly `[]`); PDF omits `charts`.
- TDD: failing test first, then minimal implementation. No `unwrap()` in library code outside tests.
- `cargo fmt` and `cargo clippy -p <crate> -- -D warnings` clean for touched crates; periodically `--workspace`.
- Skills + `demo/README.md` smoke after **each** wave (same standing gate as format-parity).
- Fixture generation via `cargo run -p dotall-cli --example generate_demos` when demos need a comment/logo/chart.
- Branch `feat/office-parity-atlas` (PR #16 against `main`). Many commits; no amend; **do not merge unless asked**.
- **Controller commits only.** Implementers (parallel format subagents) must **NOT** `git add` / `git commit` / `git push`. Parallel agents collide on `.git/index.lock`. After a task is reviewed, the controller commits with the exact `git add` paths and message in that task.
- Do not implement Waves 26+, formula evaluation, chart insert, unified IR, or SDKs.
- Charts/pivots/VBA/SmartArt/theme rewrite/track-changes authoring/PDF content-stream drawing remain preserve-or-reject.

---

## Locked operation names and payloads

Discoverable via `edit_capabilities`. Agents must still call `capabilities`/`inspect` at runtime; these names are what this plan ships.

| Op | Payload | Reject |
|----|---------|--------|
| `insert_comment` (xlsx) | `{ "sheet", "address", "author", "text" }` | Missing cell; empty text/author; cell already has a comment; `delete_comment` / `set_comment` / targeting existing `element_id` |
| `insert_comment` (pptx) | `{ "slide", "author", "text", "shape"? }` | Missing slide; empty text/author; `shape` not on slide when provided |
| `insert_comment` (docx) | `{ "index" \| "element_id", "author", "text" }` | Missing/uneditable paragraph; empty text/author |
| `insert_comment` (pdf) | `{ "page", "author", "text", "rect"? }` | Missing page; empty text; encrypted/signed; default `rect` `[50, 50, 70, 70]` if omitted |
| `insert_picture` (xlsx) | `{ "sheet", "from_cell", "bytes_base64", "content_type"? }` | Missing sheet; invalid base64; non `image/png` or `image/jpeg`; empty bytes. `content_type` defaults to `image/png` |
| `insert_picture` (pptx) | `{ "slide", "bytes_base64", "content_type"?, "name"? }` | Missing slide; invalid image. Optional `name` defaults to `Picture N` |
| `insert_picture` (docx) | `{ "index" \| "element_id", "bytes_base64", "content_type"? }` | Missing/uneditable paragraph; invalid image |
| `insert_picture` (pdf) | `{ "page", "bytes_base64", "content_type"?, "rect"? }` | Missing page; invalid image; encrypted/signed. Default `rect` `[400, 700, 500, 780]`. Must **not** rewrite page `/Contents` |
| `set_cell_font` (xlsx) | `{ "sheet", "address", "bold"?, "italic"?, "name"?, "size_pt"?, "color"? }` | Missing cell; empty payload (at least one font field required); invalid `#RRGGBB`/`RRGGBB` color |
| `set_cell_fill` (xlsx) | `{ "sheet", "address", "color" }` | Missing cell; `color` must be `#RRGGBB`/`RRGGBB` or `null` (clear pattern fill back to none) |
| `set_table_cell_italic` (pptx) | `{ "slide", "table", "row", "col", "italic" }` | Missing slide/table; out-of-range cell |
| `set_paragraph_spacing` (docx) | `{ "index" \| "element_id", "before_pt"?, "after_pt"? }` | Missing paragraph; both omitted; negative values. Writes `w:spacing` `w:before`/`w:after` in twips (`pt * 20`) |
| `insert_page_break` (docx) | `{ "index" \| "element_id" }` | Missing/uneditable paragraph. Inserts `<w:r><w:br w:type="page"/></w:r>` as the first run of the paragraph |
| `rotate_page` (pdf) | `{ "page", "degrees" }` | Missing page; `degrees` not in `{0, 90, 180, 270}`; encrypted/signed. Writes page `/Rotate` |

`bytes_base64` is raw image bytes, standard Base64 (not data-URL). Tests use the 1×1 PNG below.

```rust
const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
    0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
    0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8,
    0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D, 0xB4, 0x00, 0x00, 0x00,
    0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];
```

---

## File map

| Area | Create | Modify |
|------|--------|--------|
| XLSX comments | `crates/dotall-xlsx/src/edits/writer/comments.rs`, `crates/dotall-xlsx/tests/insert_comment.rs` | `model.rs`, `ids.rs`, `parser.rs`, `format.rs`, `edits/ops.rs`, `edits/mod.rs`, `edits/validate.rs`, `edits/writer/mod.rs`, `edits/writer/package.rs`, `lib.rs` |
| XLSX charts inspect | `crates/dotall-xlsx/tests/inspect_charts.rs` | `model.rs`, `ids.rs`, `parser.rs`, `format.rs` |
| XLSX pictures | `crates/dotall-xlsx/src/edits/writer/pictures.rs`, `crates/dotall-xlsx/tests/insert_picture.rs` | same edit/format/parser/model/ids as comments |
| XLSX font/fill | `crates/dotall-xlsx/src/edits/writer/cell_style.rs`, `crates/dotall-xlsx/tests/set_cell_font.rs`, `crates/dotall-xlsx/tests/set_cell_fill.rs` | `ops.rs`, `mod.rs`, `validate.rs`, `package.rs`, `format.rs`, `parser.rs` (reparse `s=` + fills/fonts not required in model beyond existing `style_id`) |
| PPTX comments | `crates/dotall-pptx/tests/insert_comment.rs` (or extend `surgical_edit.rs` **only if** the file stays reviewable; prefer a dedicated test file) | `model.rs`, `ids.rs`, `parser.rs`, `format.rs`, `edits.rs`, `fixture.rs`, `lib.rs` |
| PPTX charts inspect | `crates/dotall-pptx/tests/inspect_charts.rs` | `model.rs`, `ids.rs`, `parser.rs`, `format.rs`, `fixture.rs` |
| PPTX pictures | `crates/dotall-pptx/tests/insert_picture.rs` | `model.rs`, `parser.rs`, `format.rs`, `edits.rs`, `fixture.rs` |
| PPTX italic | (extend `crates/dotall-pptx/tests/surgical_edit.rs` **or** `set_table_cell_italic.rs`) | `format.rs`, `edits.rs` |
| DOCX comments | `crates/dotall-docx/tests/insert_comment.rs` | `model.rs`, `ids.rs`, `parser.rs`, `format.rs`, `edits.rs`, `fixture.rs`, `lib.rs` |
| DOCX charts inspect | `crates/dotall-docx/tests/inspect_charts.rs` | `model.rs`, `ids.rs`, `parser.rs`, `format.rs`, `fixture.rs` |
| DOCX pictures | `crates/dotall-docx/tests/insert_picture.rs` | `model.rs`, `parser.rs`, `format.rs`, `edits.rs`, `fixture.rs` |
| DOCX spacing / break | `crates/dotall-docx/tests/set_paragraph_spacing.rs`, `crates/dotall-docx/tests/insert_page_break.rs` | `format.rs`, `edits.rs` |
| PDF comments | `crates/dotall-pdf/tests/insert_comment.rs` | `model.rs`, `ids.rs`, `parser.rs`, `format.rs`, `edits.rs`, `fixture.rs`, `lib.rs` |
| PDF stamps | `crates/dotall-pdf/tests/insert_picture.rs` | `model.rs`, `parser.rs`, `format.rs`, `edits.rs` |
| PDF rotate | `crates/dotall-pdf/tests/rotate_page.rs` | `model.rs`, `parser.rs`, `format.rs`, `edits.rs` |
| Demos / skills | — | `skills/{xlsx,pptx,docx,pdf}/SKILL.md`, `demo/README.md`, `crates/dotall-cli/examples/generate_demos.rs`, `crates/dotall-cli/src/q3_pack.rs`, `crates/dotall-{pptx,docx,pdf}/src/fixture.rs` (`demo_*` generators) |
| Shared OOXML | — | No `dotall-ooxml` API change. Format crates already `rebuild_package` with additions (`xlsx`/`pptx` explicit maps; `docx` treats unseen replacement keys as additions). |
| Spec pointer | — | `docs/superpowers/specs/2026-08-17-office-parity-atlas-design.md` §7 |

Do not touch `dotall-core` unless a generic CLI flag is required — prefer `--ops-json`.

---

## Orchestration

- Launch **four sibling Task subagents** per wave (xlsx / pptx / docx / pdf). They implement and run tests; they **must not commit**.
- Controller reviews, runs the crate test + clippy, then commits using that task’s message.
- If two format agents finish together, serialize commits (wait for `index.lock`).
- Wave N+1 for a format starts only after that format’s Wave N tests are green **and** the controller commit landed.
- Wave 23b (charts inspect) starts after Wave 23 inspect JSON exists on that Office crate (comment insert may still be in flight **if** `comments[]` already parses). Prefer: finish Wave 23 insert + Demo23, then 23b.
- After each wave’s four format tasks: Demo task (skills already updated in format tasks; regenerate demos; README smoke; workspace test).

```bash
cargo test -p <touched-crate>
cargo clippy -p <touched-crate> -- -D warnings
# after each wave:
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --check
cargo run -p dotall-cli --example generate_demos
# then CLI smoke from demo/README.md
```

---

## Wave 23 — Comment the pack (parallel xlsx / pptx / docx / pdf)

**Outcome:** `inspect` lists `comments[]`; `insert_comment` adds one new note the native app shows; existing comment parts are not rewritten by unrelated edits; edit/delete/replace of an existing comment is rejected.

---

### Task 1: XLSX inspect `comments[]` + `insert_comment`

**Files:**
- Create: `crates/dotall-xlsx/src/edits/writer/comments.rs`
- Create: `crates/dotall-xlsx/tests/insert_comment.rs`
- Modify: `crates/dotall-xlsx/src/model.rs` (add `CommentModel` + `WorkbookModel.comments`)
- Modify: `crates/dotall-xlsx/src/ids.rs` (`comment_id`)
- Modify: `crates/dotall-xlsx/src/parser.rs` (parse `xl/comments*.xml` via sheet rels)
- Modify: `crates/dotall-xlsx/src/format.rs` (inspect `comments`, capability)
- Modify: `crates/dotall-xlsx/src/edits/ops.rs` (`InsertComment`)
- Modify: `crates/dotall-xlsx/src/edits/mod.rs` (parse `insert_comment`)
- Modify: `crates/dotall-xlsx/src/edits/validate.rs` (`validate` **and** `validate_with_source`)
- Modify: `crates/dotall-xlsx/src/edits/writer/mod.rs`, `package.rs`
- Modify: `crates/dotall-xlsx/src/lib.rs` (re-export `CommentModel`)
- Modify: `skills/xlsx/SKILL.md`

**Interfaces:**
- Consumes: existing `WorkbookModel`, `worksheet_paths`, `rebuild_package(original, replacements, removals, additions)`, `Note` fixtures
- Produces:
  - `CommentModel { element_id, author, text, sheet, address }`
  - `ids::comment_id(sheet, address, schema_version) -> String` prefix `cm_`
  - inspect `summary.comments: [...]`
  - `XlsxEditOp::InsertComment { sheet, address, author, text }`
  - `comments::insert(package, sheet, address, author, text) -> Result<PatchedOutput>`

- [ ] **Step 1: Write the failing inspect + insert tests**

Create `crates/dotall-xlsx/tests/insert_comment.rs`:

```rust
use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read};

use dotall_core::SemanticOperation;
use dotall_core::registry::FormatHandler;
use dotall_xlsx::{XlsxFormat, parse_workbook};
use rust_xlsxwriter::{Note, Workbook};
use tempfile::tempdir;
use zip::ZipArchive;

fn write_note_fixture(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let inputs = workbook.add_worksheet().set_name("Inputs").expect("sheet");
    inputs.write_string(1, 0, "Rate").expect("label");
    inputs.write_number(1, 1, 0.10).expect("rate");
    inputs
        .insert_note(1, 1, &Note::new("Seeded note").set_author("Reviewer"))
        .expect("note");
    let revenue = workbook.add_worksheet().set_name("Revenue").expect("sheet");
    revenue.write_string(0, 0, "X").expect("write");
    workbook.save(path).expect("save");
}

fn write_blank_fixture(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let inputs = workbook.add_worksheet().set_name("Inputs").expect("sheet");
    inputs.write_string(1, 0, "Rate").expect("label");
    inputs.write_number(1, 1, 0.10).expect("rate");
    workbook.save(path).expect("save");
}

fn zip_entries(package: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(package)).expect("zip");
    let mut entries = BTreeMap::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).expect("entry");
        let name = entry.name().to_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).expect("read");
        entries.insert(name, bytes);
    }
    entries
}

fn assert_untouched_parts(before: &[u8], after: &[u8], patched: &[&str]) {
    let before_entries = zip_entries(before);
    let after_entries = zip_entries(after);
    for (name, before_bytes) in &before_entries {
        if patched.contains(&name.as_str()) {
            continue;
        }
        let after_bytes = after_entries
            .get(name)
            .unwrap_or_else(|| panic!("entry retained: {name}"));
        assert_eq!(before_bytes, after_bytes, "bytes changed for {name}");
    }
}

#[test]
fn inspect_lists_legacy_note_comments() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_note_fixture(&source);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let inspection = handler.inspect(&model).expect("inspect");
    let comments = inspection.summary["comments"]
        .as_array()
        .expect("comments array");
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0]["sheet"], "Inputs");
    assert_eq!(comments[0]["address"], "B2");
    assert_eq!(comments[0]["author"], "Reviewer");
    assert_eq!(comments[0]["text"], "Seeded note");
    assert!(
        comments[0]["element_id"]
            .as_str()
            .expect("id")
            .starts_with("cm_")
    );
}

#[test]
fn inspect_empty_comments_when_workbook_has_none() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_blank_fixture(&source);
    let handler = XlsxFormat;
    let inspection = handler
        .inspect(&handler.parse(&source).expect("parse"))
        .expect("inspect");
    assert_eq!(inspection.summary["comments"].as_array().expect("arr").len(), 0);
}

#[test]
fn capabilities_advertise_insert_comment() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_blank_fixture(&source);
    let inspection = XlsxFormat
        .inspect(&XlsxFormat.parse(&source).expect("parse"))
        .expect("inspect");
    assert!(
        inspection
            .edit_capabilities
            .iter()
            .any(|cap| cap.operation == "insert_comment")
    );
}

#[test]
fn insert_comment_adds_comments_part_and_reparses() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_blank_fixture(&source);
    let before = fs::read(&source).expect("bytes");

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "insert_comment".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "address": "B2",
                    "author": "Dotall",
                    "text": "Review the rate"
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&source, &edit).expect("apply");
    let after_entries = zip_entries(&patched.bytes);
    assert!(
        after_entries.keys().any(|name| name.starts_with("xl/comments")),
        "expected xl/commentsN.xml"
    );
    assert!(
        after_entries
            .keys()
            .any(|name| name.contains("vmlDrawing") || name.contains("xl/drawings/")),
        "expected vml drawing part"
    );
    fs::write(&source, &patched.bytes).expect("rewrite");
    let parsed = parse_workbook(&source).expect("reparse");
    assert_eq!(parsed.comments.len(), 1);
    assert_eq!(parsed.comments[0].sheet, "Inputs");
    assert_eq!(parsed.comments[0].address, "B2");
    assert_eq!(parsed.comments[0].author, "Dotall");
    assert_eq!(parsed.comments[0].text, "Review the rate");
    assert_eq!(edit.semantic_diff[0].change, "insert_comment");

    let allowed = [
        "xl/worksheets/sheet1.xml",
        "xl/worksheets/_rels/sheet1.xml.rels",
        "[Content_Types].xml",
        "xl/comments1.xml",
        "xl/drawings/vmlDrawing1.vml",
        "xl/drawings/_rels/vmlDrawing1.vml.rels",
    ];
    // New parts will not exist in `before`; assert_untouched_parts only checks retained names.
    assert_untouched_parts(&before, &patched.bytes, &allowed);
}

#[test]
fn insert_comment_rejects_cell_that_already_has_a_comment() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_note_fixture(&source);
    let error = XlsxFormat
        .validate_edit(
            &XlsxFormat.parse(&source).expect("parse"),
            &[SemanticOperation {
                kind: "insert_comment".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "address": "B2",
                    "author": "Dotall",
                    "text": "Nope"
                }),
            }],
        )
        .expect_err("replace is insert-only reject");
    let message = error.to_string();
    assert!(
        message.contains("already has a comment") || message.contains("insert-only"),
        "{message}"
    );
}

#[test]
fn delete_comment_is_unsupported() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_note_fixture(&source);
    let error = XlsxFormat
        .validate_edit(
            &XlsxFormat.parse(&source).expect("parse"),
            &[SemanticOperation {
                kind: "delete_comment".into(),
                payload: serde_json::json!({ "element_id": "cm_anything" }),
            }],
        )
        .expect_err("delete must fail");
    assert!(error.to_string().contains("unsupported"));
}

#[test]
fn set_cell_value_leaves_comment_parts_byte_identical() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_note_fixture(&source);
    let before = fs::read(&source).expect("bytes");
    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let cell_id = model.payload["sheets"][0]["cells"]
        .as_array()
        .expect("cells")
        .iter()
        .find(|cell| cell["address"] == "A2")
        .expect("A2")["element_id"]
        .as_str()
        .expect("id")
        .to_owned();
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_cell_value".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "address": "A2",
                    "element_id": cell_id,
                    "value": "Rate"
                }),
            }],
        )
        .expect("validate cell");
    let patched = handler.apply_edit(&source, &edit).expect("apply");
    let comment_parts: Vec<String> = zip_entries(&before)
        .keys()
        .filter(|name| name.contains("comments") || name.contains("vmlDrawing"))
        .cloned()
        .collect();
    assert!(!comment_parts.is_empty(), "fixture must contain comment parts");
    assert_untouched_parts(
        &before,
        &patched.bytes,
        &["xl/worksheets/sheet1.xml", "xl/sharedStrings.xml"],
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p dotall-xlsx --test insert_comment -- --nocapture`

Expected: FAIL — `CommentModel` / `comments` field / `insert_comment` capability / validate kind missing (`unsupported validated edit operation`).

- [ ] **Step 3: Implement inspect + insert**

`ids.rs` — add:

```rust
pub fn comment_id(sheet_name: &str, address: &str, schema_version: u32) -> String {
    opaque_id("cm", schema_version, &[sheet_name, address])
}
```

`model.rs` — add (keep `SCHEMA_VERSION = 1`):

```rust
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CommentModel {
    pub element_id: String,
    pub author: String,
    pub text: String,
    pub sheet: String,
    pub address: String,
}
```

On `WorkbookModel`:

```rust
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<CommentModel>,
```

`parser.rs` — after building `sheets`, parse comments:

1. Read `xl/_rels/workbook.xml.rels` + `xl/workbook.xml` to map sheet name → worksheet path (existing helpers).
2. For each `xl/worksheets/_rels/sheetN.xml.rels`, find Relationship `Type` ending in `/comments`.
3. Load that `xl/commentsN.xml`. Parse `<authors><author>` list and `<comment ref="B2" authorId="0"><text>` concatenated `t` nodes.
4. Push `CommentModel { element_id: ids::comment_id(&sheet, &ref, SCHEMA_VERSION), author, text, sheet, address: ref }`.
5. Workbooks with no comments parts leave `comments: vec![]`.

`format.rs` `inspect` summary — add `"comments": workbook.comments` (serde to JSON). Always include the key.

`edit_capabilities()` — append:

```rust
        EditCapability {
            operation: "insert_comment".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description: "Insert a new Excel Note (legacy comment) on an existing cell.".into(),
            example: json!({
                "kind": "insert_comment",
                "payload": {
                    "sheet": "Inputs",
                    "address": "B2",
                    "author": "Dotall",
                    "text": "Review the rate"
                }
            }),
            safety: "Insert-only. Adds xl/commentsN.xml + vmlDrawing + worksheet legacyDrawing/rels. Rejects cells that already have a comment. Does not edit/delete existing notes. Other sheets stay byte-identical.".into(),
        },
```

`ops.rs` — add variant:

```rust
    InsertComment {
        sheet: String,
        address: String,
        author: String,
        text: String,
    },
```

`edits/mod.rs` `parse_validated_operations` — add arm:

```rust
            "insert_comment" => Ok(XlsxEditOp::InsertComment {
                sheet: required_string(operation, "sheet")?,
                address: required_string(operation, "address")?,
                author: required_string(operation, "author")?,
                text: required_string(operation, "text")?,
            }),
```

`validate.rs` — in **both** `validate` and `validate_with_source`, before the structural-row fallback:

```rust
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "insert_comment"))
    {
        return validate_insert_comment_operations(model, operations);
    }
```

```rust
fn validate_insert_comment_operations(
    model: &ArtifactEnvelope,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    if operations.len() != 1 {
        return Err(format_error(
            "insert_comment edits cannot be combined with other operations",
        ));
    }
    let workbook = decode(model)?;
    let operation = &operations[0];
    if operation.kind != "insert_comment" {
        return Err(format_error("unsupported insert_comment edit"));
    }
    let sheet_name = operation
        .payload
        .get("sheet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|sheet| !sheet.is_empty())
        .ok_or_else(|| format_error("insert_comment requires a non-empty `sheet` field"))?;
    let address = operation
        .payload
        .get("address")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|address| !address.is_empty())
        .map(|address| address.to_ascii_uppercase())
        .ok_or_else(|| format_error("insert_comment requires a non-empty `address` field"))?;
    let author = operation
        .payload
        .get("author")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|author| !author.is_empty())
        .ok_or_else(|| format_error("insert_comment requires a non-empty `author` field"))?
        .to_owned();
    let text = operation
        .payload
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| format_error("insert_comment requires a non-empty `text` field"))?
        .to_owned();
    let sheet = workbook
        .sheets
        .iter()
        .find(|sheet| sheet.name.eq_ignore_ascii_case(sheet_name))
        .ok_or_else(|| format_error(format!("worksheet `{sheet_name}` was not found")))?;
    if !sheet.cells.iter().any(|cell| cell.address == address) {
        return Err(format_error(format!(
            "cell `{address}` was not found on `{}`",
            sheet.name
        )));
    }
    if workbook.comments.iter().any(|comment| {
        comment.sheet.eq_ignore_ascii_case(&sheet.name) && comment.address == address
    }) {
        return Err(format_error(format!(
            "cell `{}!{address}` already has a comment; insert-only (no replace)",
            sheet.name
        )));
    }
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: MODEL_SCHEMA_ID.into(),
        schema_version: MODEL_SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "insert_comment".into(),
            payload: serde_json::json!({
                "sheet": sheet.name,
                "address": address,
                "author": author,
                "text": text,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{}!{address}", sheet.name),
            element_id: crate::ids::comment_id(&sheet.name, &address, MODEL_SCHEMA_VERSION),
            change: "insert_comment".into(),
            before: None,
            after: Some(text),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}
```

If `decode` still uses `SCHEMA_ID` from ops vs model, keep the existing `MODEL_SCHEMA_ID` / `MODEL_SCHEMA_VERSION` aliases already in `validate.rs`.

`writer/comments.rs` — implement `pub(super) fn insert(...)`:

1. Resolve worksheet path via existing `worksheet_paths`.
2. Choose next unused `N` for `xl/comments{N}.xml` and `xl/drawings/vmlDrawing{N}.vml`.
3. Build comments XML (escape author/text with `quick_xml::escape::escape`):

```xml
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<comments xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <authors><author>Dotall</author></authors>
  <commentList>
    <comment ref="B2" authorId="0"><text><t>Review the rate</t></text></comment>
  </commentList>
</comments>
```

4. Build a minimal VML note shape: `x:Row` / `x:Column` are **0-based** (B2 → row `1`, column `1`). Include `v:shapetype` `_x0000_t202` and `x:ClientData ObjectType="Note"`.
5. Replacements:
   - worksheet XML: insert `<legacyDrawing r:id="rIdK"/>` immediately before `</worksheet>` if missing.
   - `xl/worksheets/_rels/sheetN.xml.rels`: create from empty Relationships if missing; add comments rel (`http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments`, Target `../comments{N}.xml`) and vmlDrawing rel (`.../vmlDrawing`, Target `../drawings/vmlDrawing{N}.vml`).
   - `[Content_Types].xml`: ensure `<Default Extension="vml" ContentType="application/vnd.openxmlformats-officedocument.vmlDrawing"/>` and `<Override PartName="/xl/comments{N}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml"/>`.
6. Additions: `xl/comments{N}.xml`, `xl/drawings/vmlDrawing{N}.vml`.
7. `rebuild_package`. Do not rewrite other sheets, `styles.xml`, or existing comment parts on other sheets.

Reuse `insert_before_close` style string splicing already used in `header_footer.rs` (`find_open_tag`, insert before `</worksheet>`). Allocate relationship ids with the same `lowest_unused` / `rId` pattern as `workbook.rs` `add_sheet`.

`package.rs` — after the `SetHeaderFooter` arm:

```rust
    if let [
        XlsxEditOp::InsertComment {
            sheet,
            address,
            author,
            text,
        },
    ] = operations.as_slice()
    {
        return comments::insert(&original, sheet, address, author, text);
    }
```

Add `mod comments;` in `writer/mod.rs`.

`lib.rs` — export `CommentModel`.

Fix `WorkbookModel { ... }` literals in `validate.rs` unit tests: add `comments: Vec::new()`.

`skills/xlsx/SKILL.md` — in Discover / inspect bullets, add `summary.comments[]` (`sheet`, `address`, `author`, `text`, `element_id`). In ops list:

```json
{
  "kind": "insert_comment",
  "payload": {
    "sheet": "Inputs",
    "address": "B2",
    "author": "Dotall",
    "text": "Review the rate"
  }
}
```

State: insert-only Excel Notes; reject replace/delete; charts remain preserve-only until Wave 23b inspect.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p dotall-xlsx --test insert_comment`

Expected: PASS.

Run: `cargo test -p dotall-xlsx`

Expected: PASS (including `validate.rs` fixture structs).

Run: `cargo clippy -p dotall-xlsx -- -D warnings`

Expected: clean.

- [ ] **Step 5: Stop for controller commit**

Implementers must NOT commit.

Controller:

```bash
git add \
  crates/dotall-xlsx/src/model.rs \
  crates/dotall-xlsx/src/ids.rs \
  crates/dotall-xlsx/src/parser.rs \
  crates/dotall-xlsx/src/format.rs \
  crates/dotall-xlsx/src/lib.rs \
  crates/dotall-xlsx/src/edits/ops.rs \
  crates/dotall-xlsx/src/edits/mod.rs \
  crates/dotall-xlsx/src/edits/validate.rs \
  crates/dotall-xlsx/src/edits/writer/mod.rs \
  crates/dotall-xlsx/src/edits/writer/package.rs \
  crates/dotall-xlsx/src/edits/writer/comments.rs \
  crates/dotall-xlsx/tests/insert_comment.rs \
  skills/xlsx/SKILL.md
git commit -m "$(cat <<'EOF'
feat(xlsx): inspect comments[] and insert_comment notes

EOF
)"
```

---

### Task 2: PPTX inspect `comments[]` + `insert_comment`

**Files:**
- Create: `crates/dotall-pptx/tests/insert_comment.rs`
- Modify: `crates/dotall-pptx/src/model.rs`, `ids.rs`, `parser.rs`, `format.rs`, `edits.rs`, `fixture.rs`, `lib.rs`
- Modify: `skills/pptx/SKILL.md`

**Interfaces:**
- Consumes: `PresentationModel`, `PackagePatch { replacements, additions, removals }`, `rebuild_package`, `insert_before_close`, `add_slide_patch` rel patterns
- Produces:
  - `CommentModel { element_id, author, text, slide, shape: Option<String> }`
  - `ids::comment_id(slide_name, idx, schema_version)` prefix `cm_`
  - inspect `summary.comments`
  - validate/apply `insert_comment`

- [ ] **Step 1: Write the failing tests**

Add `pptx_with_comment()` to `fixture.rs`: copy `minimal_pptx()`, plus:

- `[Content_Types].xml` Override `/ppt/commentAuthors.xml` (`application/vnd.openxmlformats-officedocument.presentationml.commentAuthors+xml`) and `/ppt/comments/comment1.xml` (`application/vnd.openxmlformats-officedocument.presentationml.comments+xml`).
- `ppt/commentAuthors.xml`:

```xml
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:cmAuthorLst xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cmAuthor id="0" name="Reviewer" initials="R" lastIdx="1" clrIdx="0"/>
</p:cmAuthorLst>
```

- `ppt/comments/comment1.xml`:

```xml
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:cmLst xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cm authorId="0" dt="2026-08-17T00:00:00" idx="1">
    <p:pos x="0" y="0"/>
    <p:text>Seeded slide note</p:text>
  </p:cm>
</p:cmLst>
```

- `ppt/slides/_rels/slide1.xml.rels` Relationship Type `http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments` Target `../comments/comment1.xml`.

Export `pptx_with_comment` from `lib.rs`.

`crates/dotall-pptx/tests/insert_comment.rs`:

```rust
use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read};

use dotall_core::SemanticOperation;
use dotall_core::registry::FormatHandler;
use dotall_pptx::{PptxFormat, minimal_pptx, parse_presentation_bytes, pptx_with_comment};
use tempfile::tempdir;
use zip::ZipArchive;

fn zip_entries(package: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(package)).expect("zip");
    let mut entries = BTreeMap::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).expect("entry");
        let name = entry.name().to_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).expect("read");
        entries.insert(name, bytes);
    }
    entries
}

fn assert_untouched_entries_identical(before: &[u8], after: &[u8], patched: &[&str]) {
    let before_entries = zip_entries(before);
    let after_entries = zip_entries(after);
    for (name, before_bytes) in &before_entries {
        if patched.contains(&name.as_str()) {
            continue;
        }
        let after_bytes = after_entries
            .get(name)
            .unwrap_or_else(|| panic!("entry retained: {name}"));
        assert_eq!(before_bytes, after_bytes, "bytes changed for {name}");
    }
}

#[test]
fn inspect_lists_slide_comments() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    fs::write(&path, pptx_with_comment()).expect("write");
    let inspection = PptxFormat
        .inspect(&PptxFormat.parse(&path).expect("parse"))
        .expect("inspect");
    let comments = inspection.summary["comments"].as_array().expect("arr");
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0]["slide"], "Slide 1");
    assert_eq!(comments[0]["author"], "Reviewer");
    assert_eq!(comments[0]["text"], "Seeded slide note");
}

#[test]
fn inspect_empty_comments_without_parts() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    fs::write(&path, minimal_pptx()).expect("write");
    let inspection = PptxFormat
        .inspect(&PptxFormat.parse(&path).expect("parse"))
        .expect("inspect");
    assert_eq!(inspection.summary["comments"].as_array().expect("arr").len(), 0);
}

#[test]
fn insert_comment_adds_comment_part() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    let before = minimal_pptx();
    fs::write(&path, &before).expect("write");
    let handler = PptxFormat;
    let edit = handler
        .validate_edit(
            &handler.parse(&path).expect("parse"),
            &[SemanticOperation {
                kind: "insert_comment".into(),
                payload: serde_json::json!({
                    "slide": "Slide 1",
                    "author": "Dotall",
                    "text": "Check NPS"
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_presentation_bytes(&patched.bytes).expect("reparse");
    assert_eq!(after.comments.len(), 1);
    assert_eq!(after.comments[0].slide, "Slide 1");
    assert_eq!(after.comments[0].text, "Check NPS");
    assert_eq!(after.comments[0].author, "Dotall");
    assert!(zip_entries(&patched.bytes).contains_key("ppt/commentAuthors.xml"));
    assert!(
        zip_entries(&patched.bytes)
            .keys()
            .any(|name| name.starts_with("ppt/comments/comment"))
    );
    assert_untouched_entries_identical(
        &before,
        &patched.bytes,
        &[
            "[Content_Types].xml",
            "ppt/slides/_rels/slide1.xml.rels",
            "ppt/commentAuthors.xml",
            "ppt/comments/comment1.xml",
        ],
    );
}

#[test]
fn delete_comment_is_unsupported() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    fs::write(&path, pptx_with_comment()).expect("write");
    let error = PptxFormat
        .validate_edit(
            &PptxFormat.parse(&path).expect("parse"),
            &[SemanticOperation {
                kind: "delete_comment".into(),
                payload: serde_json::json!({ "element_id": "cm_x" }),
            }],
        )
        .expect_err("delete");
    assert!(error.to_string().contains("unsupported"));
}

#[test]
fn insert_comment_on_seeded_slide_does_not_rewrite_existing_cm_text() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    let before = pptx_with_comment();
    fs::write(&path, &before).expect("write");
    let seeded = zip_entries(&before)
        .get("ppt/comments/comment1.xml")
        .cloned()
        .expect("seeded comments");
    assert!(String::from_utf8_lossy(&seeded).contains("Seeded slide note"));
    let handler = PptxFormat;
    let edit = handler
        .validate_edit(
            &handler.parse(&path).expect("parse"),
            &[SemanticOperation {
                kind: "insert_comment".into(),
                payload: serde_json::json!({
                    "slide": "Slide 1",
                    "author": "Dotall",
                    "text": "Second note"
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let xml = String::from_utf8(
        zip_entries(&patched.bytes)["ppt/comments/comment1.xml"].clone(),
    )
    .expect("utf8");
    assert!(xml.contains("Seeded slide note"));
    assert!(xml.contains("Second note"));
    let after = parse_presentation_bytes(&patched.bytes).expect("reparse");
    assert_eq!(after.comments.len(), 2);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p dotall-pptx --test insert_comment`

Expected: FAIL — `pptx_with_comment` / `comments` field / `insert_comment` unsupported.

- [ ] **Step 3: Implement**

`model.rs`:

```rust
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CommentModel {
    pub element_id: String,
    pub author: String,
    pub text: String,
    pub slide: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<String>,
}
```

On `PresentationModel`:

```rust
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<CommentModel>,
```

`ids.rs`:

```rust
pub fn comment_id(slide_name: &str, idx: u32, schema_version: u32) -> String {
    let idx = idx.to_string();
    opaque_id("cm", schema_version, &[slide_name, &idx])
}
```

`parser.rs` — after loading each slide:

1. If `ppt/slides/_rels/slide{n}.xml.rels` has a comments relationship, parse that `ppt/comments/commentN.xml`.
2. Parse `ppt/commentAuthors.xml` into `id → name`.
3. For each `p:cm`, read `authorId`, `idx`, `p:text`. Optional shape: omit unless `p:pos` is later bound; Wave 23 may leave `shape: None` even when the payload supplied a shape name (store the name on insert in the comment text only if binding is expensive). If `insert_comment` received `shape`, persist it on `CommentModel.shape` by writing the shape name into a `p:extLst` **only if** that is already required — otherwise record `shape` in the model from the validated payload on reparse by matching `idx` order. **Locked:** parser does not need shape binding; `shape` is inspect-optional. Insert still accepts `shape` and rejects unknown names; the stored comment is slide-level `p:cm` (PowerPoint shows it on the slide).

`format.rs` inspect summary add `"comments": presentation.comments`.

Capability:

```rust
        EditCapability {
            operation: "insert_comment".into(),
            schema_version: SCHEMA_VERSION,
            description: "Insert a new comment on a slide (ECMA-376 p:cm).".into(),
            example: json!({
                "kind": "insert_comment",
                "payload": { "slide": "Slide 1", "author": "Dotall", "text": "Check NPS" }
            }),
            safety: "Insert-only. Adds or appends ppt/comments/commentN.xml and commentAuthors.xml. Does not rewrite other slides or media. Rejects delete/replace of existing comments.".into(),
        },
```

`edits.rs` `validate` match: add `"insert_comment" => validate_insert_comment(model, operation)`. Update the unsupported-kind message to include `insert_comment`.

```rust
fn validate_insert_comment(
    model: &PresentationModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let slide_ref = required_str(&operation.payload, "slide")?;
    let author = required_str(&operation.payload, "author")?;
    let text = required_str(&operation.payload, "text")?;
    let slide = selector::resolve_slide(model, slide_ref)
        .ok_or_else(|| format_error(format!("slide `{slide_ref}` was not found")))?;
    if let Some(shape) = operation.payload.get("shape").and_then(|v| v.as_str()) {
        let known = slide.shapes.iter().any(|s| s.name == shape)
            || slide.tables.iter().any(|t| t.name == shape);
        if !known {
            return Err(format_error(format!(
                "shape `{shape}` was not found on `{}`",
                slide.name
            )));
        }
    }
    let idx = model
        .comments
        .iter()
        .filter(|comment| comment.slide == slide.name)
        .count() as u32
        + 1;
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "insert_comment".into(),
            payload: serde_json::json!({
                "slide": slide.name,
                "author": author,
                "text": text,
                "shape": operation.payload.get("shape"),
                "part_name": slide.part_name,
                "idx": idx,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: slide.name.clone(),
            element_id: crate::ids::comment_id(&slide.name, idx, SCHEMA_VERSION),
            change: "insert_comment".into(),
            before: None,
            after: Some(text.to_owned()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}
```

`apply_bytes` arm `insert_comment`:

1. `part_name` = slide part. Rels path = `ppt/slides/_rels/slideN.xml.rels`.
2. If comments rel missing: add `ppt/comments/commentN.xml` (next unused N), Content_Types override, slide rels relationship.
3. If `ppt/commentAuthors.xml` missing: create with one `p:cmAuthor` (`id="0"`, `name` from payload, `lastIdx` = idx). If present: reuse author id matching `name` or append a new `p:cmAuthor` with next id; bump `lastIdx`.
4. If comment part exists: `insert_before_close` on `p:cmLst` with a new `<p:cm authorId="…" dt="2026-08-17T00:00:00" idx="{idx}"><p:pos x="0" y="0"/><p:text>{escaped}</p:text></p:cm>`. Existing `p:cm` inner XML stays substring-identical.
5. Do not modify `ppt/slides/slideN.xml` or media.

`skills/pptx/SKILL.md` — inspect `summary.comments[]`; example `insert_comment`; insert-only; charts remain rejected on text ops.

- [ ] **Step 4: Run tests**

Run: `cargo test -p dotall-pptx --test insert_comment`

Expected: PASS.

Run: `cargo test -p dotall-pptx` and `cargo clippy -p dotall-pptx -- -D warnings`

Expected: PASS / clean.

- [ ] **Step 5: Stop for controller commit**

```bash
git add \
  crates/dotall-pptx/src/model.rs \
  crates/dotall-pptx/src/ids.rs \
  crates/dotall-pptx/src/parser.rs \
  crates/dotall-pptx/src/format.rs \
  crates/dotall-pptx/src/edits.rs \
  crates/dotall-pptx/src/fixture.rs \
  crates/dotall-pptx/src/lib.rs \
  crates/dotall-pptx/tests/insert_comment.rs \
  skills/pptx/SKILL.md
git commit -m "$(cat <<'EOF'
feat(pptx): inspect comments[] and insert_comment

EOF
)"
```

---

### Task 3: DOCX inspect `comments[]` + `insert_comment`

**Files:**
- Create: `crates/dotall-docx/tests/insert_comment.rs`
- Modify: `crates/dotall-docx/src/model.rs`, `ids.rs`, `parser.rs`, `format.rs`, `edits.rs`, `fixture.rs`, `lib.rs`
- Modify: `skills/docx/SKILL.md`

**Interfaces:**
- Consumes: `DocumentModel`, `rebuild_package` (unseen keys = additions — same as `word/numbering.xml`), `insert_before_close`, `ensure_numbering_package_links` pattern, `resolve_body_paragraph`
- Produces: `CommentModel { element_id, author, text, index, paragraph_id }`; `ids::comment_id(index, w_id, schema_version)` prefix `cm_`

- [ ] **Step 1: Write the failing tests**

`fixture.rs` — add `docx_with_comment()`: `package()` plus `word/comments.xml`, document rel Type `http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments` Target `comments.xml`, Content_Types Override `/word/comments.xml` = `application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml`. Body paragraph 1 wrapped:

```xml
<w:commentRangeStart w:id="0"/>
<w:p><w:r><w:t>Alpha</w:t></w:r></w:p>
<w:commentRangeEnd w:id="0"/>
<w:r><w:commentReference w:id="0"/></w:r>
```

`word/comments.xml`:

```xml
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:comments xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:comment w:id="0" w:author="Reviewer" w:date="2026-08-17T00:00:00Z" w:initials="R">
    <w:p><w:r><w:t>Seeded memo note</w:t></w:r></w:p>
  </w:comment>
</w:comments>
```

Keep a second un-commented paragraph so insert can target index `1`. Export from `lib.rs`.

`crates/dotall-docx/tests/insert_comment.rs` — same zip helper as Task 2, then:

```rust
use dotall_core::SemanticOperation;
use dotall_core::registry::FormatHandler;
use dotall_docx::{DocxFormat, fixture, parse_document_bytes};
use tempfile::tempdir;

#[test]
fn inspect_lists_paragraph_comments() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::docx_with_comment()).expect("write");
    let inspection = DocxFormat
        .inspect(&DocxFormat.parse(&path).expect("parse"))
        .expect("inspect");
    let comments = inspection.summary["comments"].as_array().expect("arr");
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0]["index"], 0);
    assert_eq!(comments[0]["author"], "Reviewer");
    assert_eq!(comments[0]["text"], "Seeded memo note");
}

#[test]
fn insert_comment_creates_comments_xml() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    let before = fixture::minimal_docx();
    fs::write(&path, &before).expect("write");
    let handler = DocxFormat;
    let edit = handler
        .validate_edit(
            &handler.parse(&path).expect("parse"),
            &[SemanticOperation {
                kind: "insert_comment".into(),
                payload: serde_json::json!({
                    "index": 1,
                    "author": "Dotall",
                    "text": "Please confirm"
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_document_bytes(&patched.bytes).expect("reparse");
    assert_eq!(after.comments.len(), 1);
    assert_eq!(after.comments[0].index, 1);
    assert_eq!(after.comments[0].text, "Please confirm");
    assert!(String::from_utf8_lossy(
        &zip_entries(&patched.bytes)["word/document.xml"]
    )
    .contains("commentRangeStart"));
}

#[test]
fn insert_comment_does_not_rewrite_existing_comment_element() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    let before = fixture::docx_with_comment();
    fs::write(&path, &before).expect("write");
    let original_comments = zip_entries(&before)["word/comments.xml"].clone();
    assert!(String::from_utf8_lossy(&original_comments).contains("Seeded memo note"));
    let handler = DocxFormat;
    let edit = handler
        .validate_edit(
            &handler.parse(&path).expect("parse"),
            &[SemanticOperation {
                kind: "insert_comment".into(),
                payload: serde_json::json!({
                    "index": 1,
                    "author": "Dotall",
                    "text": "Second"
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let xml = String::from_utf8(zip_entries(&patched.bytes)["word/comments.xml"].clone())
        .expect("utf8");
    assert!(xml.contains("Seeded memo note"));
    assert!(xml.contains("Second"));
    assert_eq!(parse_document_bytes(&patched.bytes).expect("reparse").comments.len(), 2);
}

#[test]
fn delete_comment_is_unsupported() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::docx_with_comment()).expect("write");
    let error = DocxFormat
        .validate_edit(
            &DocxFormat.parse(&path).expect("parse"),
            &[SemanticOperation {
                kind: "delete_comment".into(),
                payload: serde_json::json!({ "index": 0 }),
            }],
        )
        .expect_err("delete");
    assert!(error.to_string().contains("unsupported"));
}
```

Copy `zip_entries` into this test file (same as Task 2). Include `assert_untouched_entries_identical` checking that `word/styles.xml` and media stay identical on insert.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p dotall-docx --test insert_comment`

Expected: FAIL.

- [ ] **Step 3: Implement**

`CommentModel` on `DocumentModel`. Parser: if `word/comments.xml` exists, parse `w:comment` `w:id` / `w:author` / inner `w:t`; map `w:id` to body paragraph by finding `w:commentRangeStart` preceding which `w:p` (document-order index). `ids::comment_id`.

`edits.rs` — constants:

```rust
const COMMENTS_REL_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments";
const COMMENTS_PART: &str = "word/comments.xml";
const COMMENTS_OVERRIDE: &str = concat!(
    r#"<Override PartName="/word/comments.xml" ContentType=""#,
    "application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml",
    r#""/>"#,
);
```

`validate_insert_comment`: `resolve_body_paragraph`; require `author` + `text`; reject `!editable`. Next `w:id` = max existing + 1 (or 0).

Apply (clone `apply_paragraph_bullet` / `ensure_numbering_package_links`):

1. If `word/comments.xml` missing: create with XML declaration + `w:comments` containing one `w:comment`. Add document rel + Content_Types override via `insert_before_close`.
2. If present: `insert_before_close(xml, "w:comments", &comment_xml)` — do not rewrite existing `w:comment` inner XML.
3. Patch `word/document.xml`: around the target `w:p` span (existing `paragraph_spans`), insert `<w:commentRangeStart w:id="{id}"/>` immediately before the `w:p` open tag and `<w:commentRangeEnd w:id="{id}"/><w:r><w:commentReference w:id="{id}"/></w:r>` immediately after the `w:p` close tag.
4. `rebuild_package` with replacements for `document.xml`, `comments.xml`, rels, content types as needed.

Reject `delete_comment` / `set_comment` via the existing `other =>` arm (update the allowed-ops list to mention `insert_comment`).

`skills/docx/SKILL.md` — comments inspect + insert-only; numbering.xml bullet pattern already documented — add comments.xml as another created part.

- [ ] **Step 4: Run tests**

Run: `cargo test -p dotall-docx --test insert_comment && cargo test -p dotall-docx && cargo clippy -p dotall-docx -- -D warnings`

Expected: PASS / clean.

- [ ] **Step 5: Stop for controller commit**

```bash
git add \
  crates/dotall-docx/src/model.rs \
  crates/dotall-docx/src/ids.rs \
  crates/dotall-docx/src/parser.rs \
  crates/dotall-docx/src/format.rs \
  crates/dotall-docx/src/edits.rs \
  crates/dotall-docx/src/fixture.rs \
  crates/dotall-docx/src/lib.rs \
  crates/dotall-docx/tests/insert_comment.rs \
  skills/docx/SKILL.md
git commit -m "$(cat <<'EOF'
feat(docx): inspect comments[] and insert_comment

EOF
)"
```

---

### Task 4: PDF inspect `comments[]` + `insert_comment`

**Files:**
- Create: `crates/dotall-pdf/tests/insert_comment.rs`
- Modify: `crates/dotall-pdf/src/model.rs`, `ids.rs`, `parser.rs`, `format.rs`, `edits.rs`, `fixture.rs`, `lib.rs`
- Modify: `skills/pdf/SKILL.md`

**Interfaces:**
- Consumes: `PdfDocumentModel`, `Document::load_mem`, `document.add_object`, page dict `/Annots`
- Produces: `CommentModel { element_id, author, text, page, subtype }`; `ids::comment_id(page, annot_index, schema_version)` prefix `cm_`

- [ ] **Step 1: Write the failing tests**

`fixture.rs` — add `pdf_with_text_annot()`: copy `minimal_form_pdf` page, add object:

```
9 0 obj<< /Type /Annot /Subtype /Text /Rect [50 50 70 70] /Contents (Seeded sticky) /T (Reviewer) /Name /Comment /P 3 0 R >>endobj
```

Page `/Annots` includes `9 0 R` in addition to the widget. Bump object numbers if they collide — keep widget 7/8, sticky as a new id.

```rust
#[test]
fn inspect_lists_text_annotations() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, pdf_with_text_annot()).expect("write");
    let inspection = PdfFormat
        .inspect(&PdfFormat.parse(&path).expect("parse"))
        .expect("inspect");
    let comments = inspection.summary["comments"].as_array().expect("arr");
    assert!(comments.iter().any(|c| c["subtype"] == "Text" && c["text"] == "Seeded sticky"));
    assert!(!inspection.summary.as_object().expect("obj").contains_key("charts"));
}

#[test]
fn insert_comment_adds_text_annot_without_rewriting_page_stream() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    let before = minimal_form_pdf();
    fs::write(&path, &before).expect("write");
    let before_contents = page_contents_bytes(&before);
    let handler = PdfFormat;
    let edit = handler
        .validate_edit(
            &handler.parse(&path).expect("parse"),
            &[SemanticOperation {
                kind: "insert_comment".into(),
                payload: serde_json::json!({
                    "page": 1,
                    "author": "Dotall",
                    "text": "Needs signature"
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_pdf_bytes(&patched.bytes).expect("reparse");
    assert!(after.comments.iter().any(|c| c.text == "Needs signature" && c.subtype == "Text"));
    assert_eq!(page_contents_bytes(&patched.bytes), before_contents);
}

#[test]
fn delete_comment_is_unsupported() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, pdf_with_text_annot()).expect("write");
    let error = PdfFormat
        .validate_edit(
            &PdfFormat.parse(&path).expect("parse"),
            &[SemanticOperation {
                kind: "delete_comment".into(),
                payload: serde_json::json!({ "element_id": "cm_x" }),
            }],
        )
        .expect_err("delete");
    assert!(error.to_string().contains("unsupported"));
}
```

Include this helper in the test file (and reuse it in Wave 24 stamp tests):

```rust
fn page_contents_bytes(package: &[u8]) -> Vec<u8> {
    let document = lopdf::Document::load_mem(package).expect("pdf");
    let pages = document.get_pages();
    let page_id = *pages.get(&1).expect("page 1");
    let page = document.get_object(page_id).expect("page obj").as_dict().expect("dict");
    let contents = page.get(b"Contents").expect("Contents");
    match contents {
        lopdf::Object::Reference(id) => document
            .get_object(*id)
            .expect("stream")
            .as_stream()
            .expect("stream")
            .content
            .clone(),
        other => panic!("unexpected Contents: {other:?}"),
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p dotall-pdf --test insert_comment`

Expected: FAIL.

- [ ] **Step 3: Implement**

`PdfPageModel` may stay as-is; comments live on `PdfDocumentModel.comments`. Parser: for each page, iterate `/Annots`; skip `/Subtype /Widget`; for `/Text` (and later `/Stamp` in Wave 24) push `CommentModel`. `subtype` string without leading slash (`Text`). `author` from `/T`, `text` from `/Contents`.

`validate_insert_comment`: encrypted/signed already rejected at `validate` top; resolve page number; require author+text; optional `rect` array of 4 numbers else default.

`apply_bytes` arm: `document.add_object` Dictionary:

```rust
    let mut dict = Dictionary::new();
    dict.set("Type", "Annot");
    dict.set("Subtype", "Text");
    dict.set("Contents", Object::string_literal(text));
    dict.set("T", Object::string_literal(author));
    dict.set("Name", "Comment");
    dict.set("Rect", vec![
        Object::Real(rect[0]),
        Object::Real(rect[1]),
        Object::Real(rect[2]),
        Object::Real(rect[3]),
    ]);
    dict.set("P", Object::Reference(page_id));
    let annot_id = document.add_object(Object::Dictionary(dict));
```

Append `Object::Reference(annot_id)` to the page `/Annots` array (create `/Annots` if missing). Do not call any content-stream writer. Do not mutate existing annot dictionaries.

Update unsupported-kind message to include `insert_comment`.

`skills/pdf/SKILL.md` — annotations inspect; insert-only sticky; no page drawing.

- [ ] **Step 4: Run tests**

Run: `cargo test -p dotall-pdf --test insert_comment && cargo test -p dotall-pdf && cargo clippy -p dotall-pdf -- -D warnings`

Expected: PASS / clean.

- [ ] **Step 5: Stop for controller commit**

```bash
git add \
  crates/dotall-pdf/src/model.rs \
  crates/dotall-pdf/src/ids.rs \
  crates/dotall-pdf/src/parser.rs \
  crates/dotall-pdf/src/format.rs \
  crates/dotall-pdf/src/edits.rs \
  crates/dotall-pdf/src/fixture.rs \
  crates/dotall-pdf/src/lib.rs \
  crates/dotall-pdf/tests/insert_comment.rs \
  skills/pdf/SKILL.md
git commit -m "$(cat <<'EOF'
feat(pdf): inspect comments[] and insert_comment text annots

EOF
)"
```

---

### Task 5: Wave 23 demo smoke (controller)

**Files:**
- Modify: `crates/dotall-cli/examples/generate_demos.rs` (`write_financials`: `insert_note` on Inputs B2)
- Modify: `crates/dotall-pptx/src/fixture.rs` `demo_deck_pptx` — include commentAuthors + comment1 + slide1 rel (same XML as `pptx_with_comment`)
- Modify: `crates/dotall-docx/src/fixture.rs` `demo_memo_docx` — comments.xml anchored to body paragraph 1
- Modify: `crates/dotall-pdf/src/fixture.rs` `demo_form_pdf` — one `/Text` annot
- Modify: `crates/dotall-cli/src/q3_pack.rs` — note on Rate (Inputs B2) for the Q3 workbook
- Modify: `demo/README.md` — Wave 23 CLI blocks for all four formats
- Modify: `skills/*/SKILL.md` only if Task 1–4 missed a sentence

**Interfaces:**
- Consumes: Wave 23 insert ops
- Produces: regenerated `demo/*.xlsx|pptx|docx|pdf` and `demo/q3-pack/*`

- [ ] **Step 1: Seed fixtures**

In `write_financials` after writing Rate:

```rust
    use rust_xlsxwriter::Note;
    inputs.insert_note(
        1,
        1,
        &Note::new("Board: confirm 10% rate").set_author("Dotall"),
    )?;
```

Mirror on Q3 Inputs B2.

- [ ] **Step 2: Regenerate**

Run: `cargo run -p dotall-cli --example generate_demos`

Expected: writes demo files without error.

- [ ] **Step 3: Append README smoke** (after each format’s Wave 22 block)

XLSX:

```bash
# Wave 23: insert_comment on Revenue!A1 (blank cell label), inspect comments[]
$DOTALL inspect demo/financials.xlsx
# expect summary.comments[] including Inputs!B2 seeded note
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"insert_comment","payload":{"sheet":"Revenue","address":"A1","author":"Dotall","text":"Labels look good"}}]'
$DOTALL apply demo/financials.xlsx --all
$DOTALL inspect demo/financials.xlsx
# expect a second comment on Revenue!A1
```

PPTX:

```bash
$DOTALL inspect demo/deck.pptx
# expect summary.comments[] (seeded slide comment)
$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"insert_comment","payload":{"slide":"Slide 1","author":"Dotall","text":"Check NPS"}}]'
$DOTALL apply demo/deck.pptx --all
$DOTALL inspect demo/deck.pptx
```

DOCX:

```bash
$DOTALL inspect demo/memo.docx
# expect summary.comments[] (seeded paragraph comment)
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"insert_comment","payload":{"index":2,"author":"Dotall","text":"Please confirm"}}]'
$DOTALL apply demo/memo.docx --all
$DOTALL inspect demo/memo.docx
```

PDF:

```bash
$DOTALL inspect demo/form.pdf
# expect summary.comments[] (seeded Text annot); no summary.charts key
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"insert_comment","payload":{"page":1,"author":"Dotall","text":"Needs signature"}}]'
$DOTALL apply demo/form.pdf --all
$DOTALL inspect demo/form.pdf
```

Prior v0 (must still work after Wave 23):

```bash
$DOTALL edit demo/financials.xlsx --op set_cell_value --sheet Inputs --address B2 --value 0.15
$DOTALL apply demo/financials.xlsx --all
$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"set_shape_text","payload":{"slide":"Slide 1","shape":"Title","text":"Q3 Review (Updated)"}}]'
$DOTALL apply demo/deck.pptx --all
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"set_paragraph_text","payload":{"index":2,"text":"Please update the status table below before Friday standup."}}]'
$DOTALL apply demo/memo.docx --all
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field","payload":{"name":"Name","value":"Grace Hopper"}}]'
$DOTALL apply demo/form.pdf --all
```

- [ ] **Step 4: Run workspace gates**

Run: `cargo test --workspace && cargo clippy --workspace -- -D warnings && cargo fmt --check`

Expected: PASS.

- [ ] **Step 5: Controller commit**

```bash
git add \
  crates/dotall-cli/examples/generate_demos.rs \
  crates/dotall-cli/src/q3_pack.rs \
  crates/dotall-pptx/src/fixture.rs \
  crates/dotall-docx/src/fixture.rs \
  crates/dotall-pdf/src/fixture.rs \
  demo/README.md \
  demo/financials.xlsx \
  demo/deck.pptx \
  demo/memo.docx \
  demo/form.pdf \
  demo/q3-pack
git commit -m "$(cat <<'EOF'
docs(demo): Wave 23 comment inspect/insert smoke

EOF
)"
```

Do not add `.all/`, `.cursor/`, or `demo/saas_model.xlsx`.

---

## Wave 23b — Inspect `charts[]` (Office only; read-only)

**Outcome:** agents list charts without unzipping. Any `insert_chart` / `set_chart_*` / `delete_chart` rejects. PDF stays without `charts`. Existing structural rejects that mention charts stay.

---

### Task 6: XLSX inspect `charts[]` + reject mutate

**Files:**
- Create: `crates/dotall-xlsx/tests/inspect_charts.rs`
- Modify: `crates/dotall-xlsx/src/model.rs`, `ids.rs`, `parser.rs`, `format.rs`
- Modify: `skills/xlsx/SKILL.md`

**Interfaces:**
- Consumes: ZIP `xl/charts/chartN.xml`, drawing anchors, worksheet chart rels
- Produces: `ChartModel { element_id, title, sheet, anchor: Option<String> }`; `ids::chart_id(sheet, part_name, schema_version)` prefix `ch_`

- [ ] **Step 1: Write the failing test**

```rust
use rust_xlsxwriter::{Chart, ChartType, Workbook};
use tempfile::tempdir;
use dotall_core::SemanticOperation;
use dotall_core::registry::FormatHandler;
use dotall_xlsx::XlsxFormat;

fn write_chart_fixture(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet().set_name("Revenue").expect("sheet");
    sheet.write_number(1, 1, 100.0).expect("v");
    sheet.write_number(2, 1, 150.0).expect("v");
    let mut chart = Chart::new(ChartType::Column);
    chart.set_title("Revenue");
    chart.add_series().set_values(("Revenue", 1, 1, 2, 1));
    sheet.insert_chart(0, 3, &chart).expect("chart");
    workbook.save(path).expect("save");
}

#[test]
fn inspect_lists_charts_with_title() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("chart.xlsx");
    write_chart_fixture(&source);
    let inspection = XlsxFormat
        .inspect(&XlsxFormat.parse(&source).expect("parse"))
        .expect("inspect");
    let charts = inspection.summary["charts"].as_array().expect("charts");
    assert_eq!(charts.len(), 1);
    assert_eq!(charts[0]["sheet"], "Revenue");
    assert_eq!(charts[0]["title"], "Revenue");
    assert!(charts[0]["element_id"].as_str().expect("id").starts_with("ch_"));
}

#[test]
fn insert_chart_is_unsupported() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("chart.xlsx");
    write_chart_fixture(&source);
    let error = XlsxFormat
        .validate_edit(
            &XlsxFormat.parse(&source).expect("parse"),
            &[SemanticOperation {
                kind: "insert_chart".into(),
                payload: serde_json::json!({ "sheet": "Revenue" }),
            }],
        )
        .expect_err("insert_chart");
    assert!(error.to_string().contains("unsupported"));
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p dotall-xlsx --test inspect_charts`

Expected: FAIL (`charts` missing).

- [ ] **Step 3: Implement**

Add `ChartModel` + `WorkbookModel.charts`. Parser: list ZIP names starting `xl/charts/chart`; parse `<c:title>` rich text (concat `a:t`); map to sheet via `xl/drawings/_rels/drawingN.xml.rels` chart rel + worksheet drawing rel. `anchor` = two-cell or `from` marker `A1` if cheap; else omit. Inspect `"charts": workbook.charts`. Empty workbook → `[]`. Do not add an insert capability. `skills/xlsx/SKILL.md`: inspect `charts[]` read-only; never unzip `xl/charts`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p dotall-xlsx --test inspect_charts && cargo test -p dotall-xlsx && cargo clippy -p dotall-xlsx -- -D warnings`

Expected: PASS.

- [ ] **Step 5: Controller commit**

```bash
git add crates/dotall-xlsx/src/model.rs crates/dotall-xlsx/src/ids.rs crates/dotall-xlsx/src/parser.rs crates/dotall-xlsx/src/format.rs crates/dotall-xlsx/tests/inspect_charts.rs skills/xlsx/SKILL.md
git commit -m "$(cat <<'EOF'
feat(xlsx): inspect charts[] read-only

EOF
)"
```

---

### Task 7: PPTX inspect `charts[]` + reject mutate

**Files:**
- Create: `crates/dotall-pptx/tests/inspect_charts.rs`
- Modify: `model.rs`, `ids.rs`, `parser.rs`, `format.rs`, `fixture.rs`, `lib.rs`, `skills/pptx/SKILL.md`

**Interfaces:**
- Consumes: slide `p:graphicFrame` + `c:chart` rel in `ppt/slides/_rels/slideN.xml.rels` targeting `ppt/charts/chartN.xml`
- Produces: `ChartModel { element_id, title, slide, name: Option<String> }`

- [ ] **Step 1: Write the failing test**

In `fixture.rs` add `pptx_with_chart()` that builds a one-slide package with a `p:graphicFrame` whose `a:graphicData` uri is `http://schemas.openxmlformats.org/drawingml/2006/chart` and child `<c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:id="rId1"/>`.

`ppt/slides/_rels/slide1.xml.rels`:

```xml
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/chart1.xml"/>
</Relationships>
```

`ppt/charts/chart1.xml`:

```xml
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <c:chart>
    <c:title><c:tx><c:rich><a:p><a:r><a:t>Pipeline</a:t></a:r></a:p></c:rich></c:tx></c:title>
    <c:plotArea/>
  </c:chart>
</c:chartSpace>
```

Content_Types Override `/ppt/charts/chart1.xml` = `application/vnd.openxmlformats-officedocument.drawingml.chart+xml`. Export `pptx_with_chart` from `lib.rs`.

`crates/dotall-pptx/tests/inspect_charts.rs`:

```rust
use std::fs;

use dotall_core::SemanticOperation;
use dotall_core::registry::FormatHandler;
use dotall_pptx::{PptxFormat, minimal_pptx, pptx_with_chart};
use tempfile::tempdir;

#[test]
fn inspect_lists_slide_charts() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    fs::write(&path, pptx_with_chart()).expect("write");
    let inspection = PptxFormat
        .inspect(&PptxFormat.parse(&path).expect("parse"))
        .expect("inspect");
    let charts = inspection.summary["charts"].as_array().expect("charts");
    assert_eq!(charts.len(), 1);
    assert_eq!(charts[0]["slide"], "Slide 1");
    assert_eq!(charts[0]["title"], "Pipeline");
    assert!(charts[0]["element_id"].as_str().expect("id").starts_with("ch_"));
}

#[test]
fn inspect_empty_charts_without_chart_parts() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    fs::write(&path, minimal_pptx()).expect("write");
    let inspection = PptxFormat
        .inspect(&PptxFormat.parse(&path).expect("parse"))
        .expect("inspect");
    assert_eq!(inspection.summary["charts"].as_array().expect("arr").len(), 0);
}

#[test]
fn insert_chart_is_unsupported() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    fs::write(&path, pptx_with_chart()).expect("write");
    let error = PptxFormat
        .validate_edit(
            &PptxFormat.parse(&path).expect("parse"),
            &[SemanticOperation {
                kind: "insert_chart".into(),
                payload: serde_json::json!({ "slide": "Slide 1" }),
            }],
        )
        .expect_err("insert_chart");
    assert!(error.to_string().contains("unsupported"));
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p dotall-pptx --test inspect_charts`

Expected: FAIL.

- [ ] **Step 3: Implement**

While parsing slides, if graphicFrame is a chart (not `a:tbl`), do not add a `TableModel`; instead load chart part title. Inspect `summary.charts`. Skill: charts inspect-only; text ops still reject graphicFrame charts.

- [ ] **Step 4: Run tests**

Run: `cargo test -p dotall-pptx --test inspect_charts && cargo test -p dotall-pptx && cargo clippy -p dotall-pptx -- -D warnings`

Expected: PASS.

- [ ] **Step 5: Controller commit**

```bash
git add crates/dotall-pptx/src/model.rs crates/dotall-pptx/src/ids.rs crates/dotall-pptx/src/parser.rs crates/dotall-pptx/src/format.rs crates/dotall-pptx/src/fixture.rs crates/dotall-pptx/src/lib.rs crates/dotall-pptx/tests/inspect_charts.rs skills/pptx/SKILL.md
git commit -m "$(cat <<'EOF'
feat(pptx): inspect charts[] read-only

EOF
)"
```

---

### Task 8: DOCX inspect `charts[]` + reject mutate

**Files:**
- Create: `crates/dotall-docx/tests/inspect_charts.rs`
- Modify: `model.rs`, `ids.rs`, `parser.rs`, `format.rs`, `fixture.rs`, `lib.rs`, `skills/docx/SKILL.md`

**Interfaces:**
- Consumes: `word/charts/chartN.xml` + document rels `.../chart`
- Produces: `ChartModel { element_id, title, paragraph: Option<u32> }`

- [ ] **Step 1: Write the failing test**

In `fixture.rs` add `docx_with_chart()` using `package()` plus `word/charts/chart1.xml`, a document rel Type `http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart` Target `charts/chart1.xml`, and a body paragraph containing a `w:drawing` / `c:chart r:id="rId1"`. Chart part title `Mix` via `c:title` / `a:t`. Content_Types Override `/word/charts/chart1.xml` = `application/vnd.openxmlformats-officedocument.drawingml.chart+xml`. Export from `lib.rs`.

`crates/dotall-docx/tests/inspect_charts.rs`:

```rust
use std::fs;

use dotall_core::SemanticOperation;
use dotall_core::registry::FormatHandler;
use dotall_docx::{DocxFormat, fixture};
use tempfile::tempdir;

#[test]
fn inspect_lists_document_charts() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::docx_with_chart()).expect("write");
    let inspection = DocxFormat
        .inspect(&DocxFormat.parse(&path).expect("parse"))
        .expect("inspect");
    let charts = inspection.summary["charts"].as_array().expect("charts");
    assert_eq!(charts.len(), 1);
    assert_eq!(charts[0]["title"], "Mix");
    assert!(charts[0]["element_id"].as_str().expect("id").starts_with("ch_"));
}

#[test]
fn inspect_empty_charts_without_chart_parts() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write");
    let inspection = DocxFormat
        .inspect(&DocxFormat.parse(&path).expect("parse"))
        .expect("inspect");
    assert_eq!(inspection.summary["charts"].as_array().expect("arr").len(), 0);
}

#[test]
fn insert_chart_is_unsupported() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::docx_with_chart()).expect("write");
    let error = DocxFormat
        .validate_edit(
            &DocxFormat.parse(&path).expect("parse"),
            &[SemanticOperation {
                kind: "insert_chart".into(),
                payload: serde_json::json!({ "index": 0 }),
            }],
        )
        .expect_err("insert_chart");
    assert!(error.to_string().contains("unsupported"));
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p dotall-docx --test inspect_charts`

Expected: FAIL.

- [ ] **Step 3: Implement**

Parse chart parts from document rels; title from `c:title`; optional paragraph index if the drawing sits inside a known `w:p`. Inspect `summary.charts`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p dotall-docx --test inspect_charts && cargo test -p dotall-docx && cargo clippy -p dotall-docx -- -D warnings`

Expected: PASS.

- [ ] **Step 5: Controller commit**

```bash
git add crates/dotall-docx/src/model.rs crates/dotall-docx/src/ids.rs crates/dotall-docx/src/parser.rs crates/dotall-docx/src/format.rs crates/dotall-docx/src/fixture.rs crates/dotall-docx/src/lib.rs crates/dotall-docx/tests/inspect_charts.rs skills/docx/SKILL.md
git commit -m "$(cat <<'EOF'
feat(docx): inspect charts[] read-only

EOF
)"
```

No PDF charts task. No extra demo commit required unless generate_demos grows a chart — prefer seeding a column chart on `financials.xlsx` Revenue in the **Wave 24 demo** if Wave 23b did not regenerate demos. If Task 6’s rust_xlsxwriter chart should appear in `demo/financials.xlsx`, fold that into Task 9’s generate_demos (controller). Until then, inspect tests are sufficient.

---

## Wave 24 — Stamp a logo (parallel)

**Outcome:** `pictures[]` listed; one new image inserted; existing `ppt/media/*`, `word/media/*`, `xl/media/*` bytes unchanged; PDF stamp does not rewrite page content streams.

---

### Task 9: XLSX inspect `pictures[]` + `insert_picture`

**Files:**
- Create: `crates/dotall-xlsx/src/edits/writer/pictures.rs`
- Create: `crates/dotall-xlsx/tests/insert_picture.rs`
- Modify: `model.rs`, `ids.rs`, `parser.rs`, `format.rs`, `ops.rs`, `edits/mod.rs`, `validate.rs` (both entry points), `writer/mod.rs`, `package.rs`, `lib.rs`, `skills/xlsx/SKILL.md`

**Interfaces:**
- Consumes: drawing + media parts, `rebuild_package` additions, `TINY_PNG`
- Produces: `PictureModel { element_id, sheet, name, content_type, from_cell: Option<String> }`; `ids::picture_id`; `XlsxEditOp::InsertPicture { sheet, from_cell, bytes, content_type }`

- [ ] **Step 1: Write the failing tests**

```rust
use rust_xlsxwriter::{Image, Workbook};
use tempfile::tempdir;
use dotall_core::SemanticOperation;
use dotall_core::registry::FormatHandler;
use dotall_xlsx::{XlsxFormat, parse_workbook};

const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
    0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
    0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8,
    0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D, 0xB4, 0x00, 0x00, 0x00,
    0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

fn write_image_fixture(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet().set_name("Inputs").expect("sheet");
    sheet.write_string(0, 0, "Logo").expect("write");
    let image = Image::new_from_buffer(TINY_PNG).expect("image");
    sheet.insert_image(0, 2, &image).expect("insert");
    workbook.save(path).expect("save");
}

fn write_blank(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    workbook
        .add_worksheet()
        .set_name("Inputs")
        .expect("sheet")
        .write_string(0, 0, "A")
        .expect("write");
    workbook.save(path).expect("save");
}

#[test]
fn inspect_lists_pictures() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("logo.xlsx");
    write_image_fixture(&source);
    let pictures = XlsxFormat
        .inspect(&XlsxFormat.parse(&source).expect("parse"))
        .expect("inspect")
        .summary["pictures"]
        .as_array()
        .expect("arr")
        .clone();
    assert_eq!(pictures.len(), 1);
    assert_eq!(pictures[0]["sheet"], "Inputs");
    assert!(pictures[0]["content_type"].as_str().expect("ct").contains("png"));
}

#[test]
fn insert_picture_adds_media_and_leaves_other_sheets_identical() {
    use std::fs;
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("logo.xlsx");
    write_blank(&source);
    let before = fs::read(&source).expect("bytes");
    let encoded = base64::engine::general_purpose::STANDARD.encode(TINY_PNG);
    let handler = XlsxFormat;
    let edit = handler
        .validate_edit(
            &handler.parse(&source).expect("parse"),
            &[SemanticOperation {
                kind: "insert_picture".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "from_cell": "A1",
                    "bytes_base64": encoded,
                    "content_type": "image/png"
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&source, &edit).expect("apply");
    fs::write(&source, &patched.bytes).expect("rewrite");
    assert_eq!(parse_workbook(&source).expect("reparse").pictures.len(), 1);
    assert!(zip_entries(&patched.bytes)
        .keys()
        .any(|name| name.starts_with("xl/media/")));
}

#[test]
fn insert_picture_does_not_rewrite_existing_media_bytes() {
    use std::fs;
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("logo.xlsx");
    write_image_fixture(&source);
    let before = fs::read(&source).expect("bytes");
    let existing: Vec<(String, Vec<u8>)> = zip_entries(&before)
        .into_iter()
        .filter(|(name, _)| name.starts_with("xl/media/"))
        .collect();
    assert!(!existing.is_empty());
    let encoded = base64::engine::general_purpose::STANDARD.encode(TINY_PNG);
    let handler = XlsxFormat;
    let edit = handler
        .validate_edit(
            &handler.parse(&source).expect("parse"),
            &[SemanticOperation {
                kind: "insert_picture".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "from_cell": "A2",
                    "bytes_base64": encoded
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&source, &edit).expect("apply");
    let after = zip_entries(&patched.bytes);
    for (name, bytes) in existing {
        assert_eq!(after.get(&name).expect("retained media"), &bytes);
    }
}

#[test]
fn replace_picture_is_unsupported() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("logo.xlsx");
    write_image_fixture(&source);
    let error = XlsxFormat
        .validate_edit(
            &XlsxFormat.parse(&source).expect("parse"),
            &[SemanticOperation {
                kind: "replace_picture".into(),
                payload: serde_json::json!({ "element_id": "pic_x" }),
            }],
        )
        .expect_err("replace");
    assert!(error.to_string().contains("unsupported"));
}
```

Add `base64` as a **dev-dependency** of `dotall-xlsx` (`base64 = "0.22"`) — tests only. Library code may decode with the same crate as a normal dependency if validate lives in the lib (prefer decode in `edits/mod.rs` / validate using `base64` **library** dependency). Locked: add `base64 = "0.22"` to `[dependencies]` of `dotall-xlsx` (small, used by validate+writer).

Copy `zip_entries` from Task 1 into this test file.

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p dotall-xlsx --test insert_picture`

Expected: FAIL.

- [ ] **Step 3: Implement**

`PictureModel` + parse: for each worksheet drawing rel (`.../drawing`), parse `xdr:twoCellAnchor`/`xdr:from` col/row → A1; `a:blip r:embed` → drawing rels → `xl/media/imageN.png`; content-type from Content_Types Default/Override.

`insert_picture` writer (`pictures.rs`):

1. Decode base64; reject empty; allow `image/png` and `image/jpeg` only.
2. Add `xl/media/image{N}.png` (or `.jpeg`).
3. If the sheet has no drawing: create `xl/drawings/drawing{N}.xml` with a single `xdr:oneCellAnchor` at `from_cell` (convert A1 to 0-based col/row), `a:blip r:embed="rId1"`, cx/cy 1cm EMUs (`914400`). Add worksheet rel drawing, Content_Types Override `application/vnd.openxmlformats-officedocument.drawing+xml`, Default `png` if missing. Worksheet XML `<drawing r:id="..."/>` before `legacyDrawing` / `</worksheet>`.
4. If the sheet already has a drawing: **append** a new `xdr:oneCellAnchor` to that drawing XML (existing anchors substring-identical) and a new media rel on `xl/drawings/_rels/drawingN.xml.rels`. Do not rewrite existing `xl/media/*` bytes.
5. Reject `replace_picture` / `delete_picture` / `set_picture` as unsupported kinds.

Validate: sheet exists; `from_cell` parses as A1; decode base64 in validate so apply receives raw bytes in the canonical payload (`bytes_base64` may remain in the SemanticOperation payload).

Capability example uses a short base64 string of `TINY_PNG`.

Skill: insert-only; existing drawings byte-identical.

- [ ] **Step 4: Run tests**

Run: `cargo test -p dotall-xlsx --test insert_picture && cargo test -p dotall-xlsx && cargo clippy -p dotall-xlsx -- -D warnings`

Expected: PASS.

- [ ] **Step 5: Controller commit**

```bash
git add crates/dotall-xlsx/Cargo.toml crates/dotall-xlsx/src crates/dotall-xlsx/tests/insert_picture.rs skills/xlsx/SKILL.md
git commit -m "$(cat <<'EOF'
feat(xlsx): inspect pictures[] and insert_picture

EOF
)"
```

---

### Task 10: PPTX inspect `pictures[]` + `insert_picture`

**Files:**
- Create: `crates/dotall-pptx/tests/insert_picture.rs`
- Modify: `model.rs`, `ids.rs`, `parser.rs`, `format.rs`, `edits.rs`, `fixture.rs`, `lib.rs`, `skills/pptx/SKILL.md`
- Modify: `crates/dotall-pptx/Cargo.toml` — `base64 = "0.22"`

**Interfaces:**
- Consumes: `media_parts`, `PackagePatch.additions`, `p:pic` XML, `add_textbox`-style slide splice
- Produces: `PictureModel { element_id, slide, name, part }`; insert new `p:pic` + `ppt/media/imageN.png` + slide rel

- [ ] **Step 1: Write the failing tests**

Use `minimal_pptx()` (no real PNG today — `logo.bin` is random). Add `pptx_with_png()`: `ppt/media/image1.png` = `TINY_PNG`, slide `p:pic` with `a:blip r:embed="rId1"`, slide rels image relationship Type `http://schemas.openxmlformats.org/officeDocument/2006/relationships/image`, Content_Types Default `png`. Inspect `pictures[0].part` ends with `image1.png`. Insert on `minimal_pptx` adds a new media part; `minimal_pptx_with_media`’s `ppt/media/logo.bin` stays byte-identical.

Reject `replace_picture`. Capability `insert_picture`.

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p dotall-pptx --test insert_picture`

Expected: FAIL.

- [ ] **Step 3: Implement**

Parser: collect `p:pic` `p:cNvPr name` + blip embed → rel target. Model array on `PresentationModel`. Inspect `summary.pictures` (keep existing `media_parts` too).

Insert: next unused `ppt/media/imageN.png`; Default png in Content_Types; slide rels image rel; splice a `p:pic` into `p:spTree` before `</p:spTree>` (after existing shapes). EMU size 1cm×1cm; `a:off` x=0 y=0. Optional `name` uniqueness on the slide (reject duplicate like `add_textbox`). Do not modify existing `ppt/media/*`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p dotall-pptx --test insert_picture && cargo test -p dotall-pptx && cargo clippy -p dotall-pptx -- -D warnings`

Expected: PASS.

- [ ] **Step 5: Controller commit**

```bash
git add crates/dotall-pptx/Cargo.toml crates/dotall-pptx/src crates/dotall-pptx/tests/insert_picture.rs skills/pptx/SKILL.md
git commit -m "$(cat <<'EOF'
feat(pptx): inspect pictures[] and insert_picture

EOF
)"
```

---

### Task 11: DOCX inspect `pictures[]` + `insert_picture`

**Files:**
- Create: `crates/dotall-docx/tests/insert_picture.rs`
- Modify: `model.rs`, `ids.rs`, `parser.rs`, `format.rs`, `edits.rs`, `fixture.rs`, `lib.rs`, `Cargo.toml` (`base64`), `skills/docx/SKILL.md`

**Interfaces:**
- Consumes: numbering-style part create (`word/media/imageN.png` + document rels + Content_Types)
- Produces: `PictureModel { element_id, index, part }`; inline `w:drawing` / `wp:inline` / `a:blip` in the target paragraph

- [ ] **Step 1: Write the failing tests**

`docx_with_picture()`: `word/media/image1.png` = `TINY_PNG`, document rel image, inline drawing in paragraph 0. Inspect lists it. Insert into `minimal_docx` paragraph 1 creates `word/media/image1.png`. Existing media in `minimal_docx_with_media` stays identical when inserting into a body paragraph. Reject `replace_picture`.

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p dotall-docx --test insert_picture`

Expected: FAIL.

- [ ] **Step 3: Implement**

Parser: `a:blip` in `word/document.xml` + rels → media part. Insert: next image N; Default png; document rel Type `http://schemas.openxmlformats.org/officeDocument/2006/relationships/image` Target `media/imageN.png`; append inline drawing as last run in the target `w:p` (`wp:inline` extent 914400 EMUs). Clone `ensure_numbering_package_links` for rels + content types. Do not rewrite existing `word/media/*`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p dotall-docx --test insert_picture && cargo test -p dotall-docx && cargo clippy -p dotall-docx -- -D warnings`

Expected: PASS.

- [ ] **Step 5: Controller commit**

```bash
git add crates/dotall-docx/Cargo.toml crates/dotall-docx/src crates/dotall-docx/tests/insert_picture.rs skills/docx/SKILL.md
git commit -m "$(cat <<'EOF'
feat(docx): inspect pictures[] and insert_picture

EOF
)"
```

---

### Task 12: PDF inspect stamp/picture annots + `insert_picture`

**Files:**
- Create: `crates/dotall-pdf/tests/insert_picture.rs`
- Modify: `model.rs`, `parser.rs`, `format.rs`, `edits.rs`, `Cargo.toml` (`base64` if not already), `skills/pdf/SKILL.md`

**Interfaces:**
- Consumes: `/Annot /Subtype /Stamp`, `document.add_object`, page `/Annots`
- Produces: `PictureModel { element_id, page, subtype }` (stamps). `insert_picture` adds Stamp + Form XObject appearance **owned by the annot**. Page `/Contents` stream bytes unchanged.

- [ ] **Step 1: Write the failing tests**

Parser already lists `/Text` in `comments[]`. Wave 24: `/Stamp` annots appear in `summary.pictures` (not comments). Fixture with a Stamp (no AP required for inspect). Insert: `bytes_base64` of `TINY_PNG`; after apply, `pictures` has a Stamp; `page_contents_bytes` equal; existing widget annot object bytes unchanged (compare `/Annots` widget refs’ dictionaries). Reject a fake `draw_image` kind. Encrypted PDFs still reject.

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p dotall-pdf --test insert_picture`

Expected: FAIL.

- [ ] **Step 3: Implement**

Parser: `/Stamp` → `pictures[]`. `insert_picture`:

1. Decode PNG/JPEG bytes.
2. Add image XObject (`/Subtype /Image`, `/Width` `/Height` 1, `/ColorSpace /DeviceRGB`, `/BitsPerComponent 8`, stream = decoded pixels **or** for PNG store as `/Filter /DCTDecode` only for JPEG). For the 1×1 PNG, decode is heavy; **locked simpler path:** embed the PNG bytes in a Form XObject stream that is the annot `/AP /N` appearance, even if some viewers show a box without pixels — **prefer** a Form XObject wrapping an Image XObject with raw RGB `255,0,0` for the 1×1 test PNG so Preview shows a red stamp. Implement a tiny PNG decoder for 1×1 8-bit RGB only; reject other PNG variants with a clear error (`insert_picture supports PNG IHDR 8-bit RGB` / JPEG). Tests use `TINY_PNG` (IHDR 1×1 RGB).
3. Annot dict `/Subtype /Stamp`, `/Rect`, `/AP << /N formRef >>`, `/P page`.
4. Append to `/Annots`. Never update page `/Contents`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p dotall-pdf --test insert_picture && cargo test -p dotall-pdf && cargo clippy -p dotall-pdf -- -D warnings`

Expected: PASS.

- [ ] **Step 5: Controller commit**

```bash
git add crates/dotall-pdf/Cargo.toml crates/dotall-pdf/src crates/dotall-pdf/tests/insert_picture.rs skills/pdf/SKILL.md
git commit -m "$(cat <<'EOF'
feat(pdf): inspect pictures[] and insert stamp annotations

EOF
)"
```

---

### Task 13: Wave 24 demo smoke (controller)

**Files:** `generate_demos.rs`, `q3_pack.rs`, format `demo_*` fixtures, `demo/README.md`

- [ ] **Step 1:** Seed a logo on `financials.xlsx` via `Image::new_from_buffer(TINY_PNG)` at Inputs C1; add PNG media + `p:pic` on `demo_deck_pptx`; inline picture on `demo_memo_docx`; optional stamp on `demo_form_pdf`. Optionally add Revenue column chart here if Wave 23b skipped demo regen.
- [ ] **Step 2:** `cargo run -p dotall-cli --example generate_demos`
- [ ] **Step 3:** README Wave 24 `insert_picture` smoke + inspect `pictures[]` + prior v0 paths.
- [ ] **Step 4:** `cargo test --workspace && cargo clippy --workspace -- -D warnings && cargo fmt --check`
- [ ] **Step 5:** Controller commit `docs(demo): Wave 24 picture/stamp smoke` with demo binaries + README + generators. Not `.cursor/` / `demo/saas_model.xlsx`.

---

## Wave 25 — Look like a report (parallel)

**Outcome:** xlsx cell font+fill; pptx table cell italic; docx spacing + page break; pdf `rotate_page`. Unrelated parts byte-identical (`styles.xml` is an allowed xlsx target).

---

### Task 14: XLSX `set_cell_font` + `set_cell_fill`

**Files:**
- Create: `crates/dotall-xlsx/src/edits/writer/cell_style.rs`
- Create: `crates/dotall-xlsx/tests/set_cell_font.rs`
- Create: `crates/dotall-xlsx/tests/set_cell_fill.rs`
- Modify: `ops.rs`, `edits/mod.rs`, `validate.rs` (both), `package.rs`, `writer/mod.rs`, `format.rs`, `skills/xlsx/SKILL.md`

**Interfaces:**
- Consumes: cell `s=` index, `xl/styles.xml` `fonts` / `fills` / `cellXfs`
- Produces: `XlsxEditOp::SetCellFont { sheet, address, bold: Option<bool>, italic: Option<bool>, name: Option<String>, size_pt: Option<f64>, color: Option<String> }`; `SetCellFill { sheet, address, color: Option<String> }` (`None` = clear to fillId 0)

- [ ] **Step 1: Write the failing tests**

`set_cell_font.rs`:

```rust
#[test]
fn set_cell_font_bold_patches_styles_and_cell_s() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source); // Inputs A1 "Hello", Revenue A1 "X" like header_footer tests
    let before = fs::read(&source).expect("bytes");
    let handler = XlsxFormat;
    let edit = handler
        .validate_edit(
            &handler.parse(&source).expect("parse"),
            &[SemanticOperation {
                kind: "set_cell_font".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "address": "A1",
                    "bold": true,
                    "name": "Calibri",
                    "size_pt": 14,
                    "color": "#1F4E79"
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&source, &edit).expect("apply");
    let styles = entry_xml(&patched.bytes, "xl/styles.xml");
    assert!(styles.contains("<b/>") || styles.contains("<b val=\"1\"/>") || styles.contains("b="));
    assert!(styles.contains("1F4E79") || styles.contains("1f4e79"));
    let sheet = entry_xml(&patched.bytes, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains("s=\""), "cell must reference a cellXf: {sheet}");
    assert_untouched_parts(
        &before,
        &patched.bytes,
        &["xl/worksheets/sheet1.xml", "xl/styles.xml"],
    );
}

#[test]
fn set_cell_font_rejects_empty_payload() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);
    let error = XlsxFormat
        .validate_edit(
            &XlsxFormat.parse(&source).expect("parse"),
            &[SemanticOperation {
                kind: "set_cell_font".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "address": "A1" }),
            }],
        )
        .expect_err("empty font");
    assert!(error.to_string().contains("font"));
}
```

Copy `write_fixture` / `entry_xml` / `assert_untouched_parts` from `tests/set_header_footer.rs`.

`set_cell_fill.rs`: payload `"color": "#FFFF00"` writes a `<patternFill patternType="solid"><fgColor rgb="FFFFFF00"/>` (or `FFFF00` with Excel `FF` alpha). `color: null` after a fill restores fillId 0 / none. Revenue sheet XML byte-identical.

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p dotall-xlsx --test set_cell_font --test set_cell_fill`

Expected: FAIL.

- [ ] **Step 3: Implement**

`cell_style.rs`:

1. Parse `fonts` count and inner XML slices; `fills`; `cellXfs` xf slices.
2. Read target cell `s` (default 0).
3. Clone that xf’s `fontId`/`fillId`/`borderId`/`xfId` / `apply*` attrs.
4. `set_cell_font`: append a `<font>` with `<b/>` if bold, `<i/>` if italic, `<sz val="14"/>`, `<color rgb="FF1F4E79"/>`, `<name val="Calibri"/>`; bump fonts count; new xf `fontId` = new index, `applyFont="1"`.
5. `set_cell_fill`: append `<fill><patternFill patternType="solid"><fgColor rgb="FFFFFF00"/></patternFill></fill>` (prepend `FF` if 6 hex digits). `null` color → `fillId="0"` `applyFill="1"`. Bump fills count.
6. Append `<xf .../>` and bump `cellXfs count`.
7. Set the cell element’s `s="{newIndex}"` (create `<c r="A1" s="N"/>` if the cell exists in the model but as a missing XML `c` — the fixture cells exist).

No theme rewrite. Do not reorder existing font/fill/xf entries (append only) so other cells’ indices stay valid.

Validate both ops: cannot combine with other kinds; sheet+address required; font needs ≥1 of bold/italic/name/size_pt/color; color hex via the same normalizer as `set_tab_color` (`FF` + RRGGBB).

Capabilities + skill examples.

Dispatch in `package.rs` like `SetHeaderFooter`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p dotall-xlsx --test set_cell_font --test set_cell_fill && cargo test -p dotall-xlsx && cargo clippy -p dotall-xlsx -- -D warnings`

Expected: PASS.

- [ ] **Step 5: Controller commit**

```bash
git add crates/dotall-xlsx/src/edits crates/dotall-xlsx/src/format.rs crates/dotall-xlsx/tests/set_cell_font.rs crates/dotall-xlsx/tests/set_cell_fill.rs skills/xlsx/SKILL.md
git commit -m "$(cat <<'EOF'
feat(xlsx): set_cell_font and set_cell_fill

EOF
)"
```

---

### Task 15: PPTX `set_table_cell_italic`

**Files:**
- Create: `crates/dotall-pptx/tests/set_table_cell_italic.rs` (or append to `surgical_edit.rs` if adding <150 lines; prefer dedicated file)
- Modify: `format.rs`, `edits.rs`, `skills/pptx/SKILL.md`

**Interfaces:**
- Consumes: `validate_set_table_cell_bold`, `patch_table_cell_bold`, `set_shape_runs_attr(cell, "b", value)`
- Produces: `set_table_cell_italic` with `italic` boolean; `set_shape_runs_attr(cell, "i", "1"|"0")`

- [ ] **Step 1: Write the failing tests**

Clone `set_table_cell_bold` tests in `surgical_edit.rs` (`pptx_with_table`, cell 0,1):

```rust
#[test]
fn set_table_cell_italic_patches_only_target_slide() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    let before = pptx_with_table();
    fs::write(&path, &before).expect("write");
    let handler = PptxFormat;
    let edit = handler
        .validate_edit(
            &handler.parse(&path).expect("parse"),
            &[SemanticOperation {
                kind: "set_table_cell_italic".into(),
                payload: serde_json::json!({
                    "slide": "Slide 1",
                    "table": "Table 1",
                    "row": 0,
                    "col": 1,
                    "italic": true
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let slide_xml = String::from_utf8(zip_entries(&patched.bytes)["ppt/slides/slide1.xml"].clone())
        .expect("xml");
    assert!(
        slide_xml.contains(r#"<a:rPr i="1"/><a:t>B1</a:t>"#)
            || slide_xml.contains(r#"i="1""#),
        "expected italic on B1: {slide_xml}"
    );
    assert!(
        !slide_xml.contains(r#"i="1""#) || slide_xml.contains("B1"),
        "must not italicize sibling cells only — check A1 unchanged"
    );
    assert_untouched_entries_identical(&before, &patched.bytes, &["ppt/slides/slide1.xml"]);
}

#[test]
fn set_table_cell_italic_rejects_out_of_range() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    fs::write(&path, pptx_with_table()).expect("write");
    let error = PptxFormat
        .validate_edit(
            &PptxFormat.parse(&path).expect("parse"),
            &[SemanticOperation {
                kind: "set_table_cell_italic".into(),
                payload: serde_json::json!({
                    "slide": "Slide 1",
                    "table": "Table 1",
                    "row": 9,
                    "col": 0,
                    "italic": true
                }),
            }],
        )
        .expect_err("oor");
    assert!(error.to_string().contains("out of range") || error.to_string().contains("row"));
}
```

Copy `zip_entries` / `assert_untouched_entries_identical` from `surgical_edit.rs` into the new test file (or use the existing tests module helpers only if they are already public — they are not; duplicate the helpers).

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p dotall-pptx --test set_table_cell_italic`

Expected: FAIL (`unsupported pptx edit`).

- [ ] **Step 3: Implement**

Duplicate `validate_set_table_cell_bold` as `validate_set_table_cell_italic` with `italic` / kind `set_table_cell_italic`. Duplicate `patch_table_cell_bold` as `patch_table_cell_italic` calling `set_shape_runs_attr(cell, "i", value)`. `apply_bytes` arm. Capability. Update unsupported-kind list. Skill: table italic; fill not shipped (spec: italic **or** fill; italic preferred).

- [ ] **Step 4: Run tests**

Run: `cargo test -p dotall-pptx --test set_table_cell_italic && cargo test -p dotall-pptx && cargo clippy -p dotall-pptx -- -D warnings`

Expected: PASS.

- [ ] **Step 5: Controller commit**

```bash
git add crates/dotall-pptx/src/format.rs crates/dotall-pptx/src/edits.rs crates/dotall-pptx/tests/set_table_cell_italic.rs skills/pptx/SKILL.md
git commit -m "$(cat <<'EOF'
feat(pptx): set_table_cell_italic

EOF
)"
```

---

### Task 16: DOCX `set_paragraph_spacing` + `insert_page_break`

**Files:**
- Create: `crates/dotall-docx/tests/set_paragraph_spacing.rs`
- Create: `crates/dotall-docx/tests/insert_page_break.rs`
- Modify: `format.rs`, `edits.rs`, `skills/docx/SKILL.md`

**Interfaces:**
- Consumes: `patch_paragraph_alignment` / `upsert_p_jc` / `paragraph_spans` / `extract_p_pr`
- Produces: `w:spacing w:before` / `w:after` in twips; `w:br w:type="page"` as first run

- [ ] **Step 1: Write the failing tests**

`set_paragraph_spacing.rs` using `fixture::minimal_docx()`:

```rust
#[test]
fn set_paragraph_spacing_writes_twips() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    let before = fixture::minimal_docx();
    fs::write(&path, &before).expect("write");
    let handler = DocxFormat;
    let edit = handler
        .validate_edit(
            &handler.parse(&path).expect("parse"),
            &[SemanticOperation {
                kind: "set_paragraph_spacing".into(),
                payload: serde_json::json!({ "index": 1, "before_pt": 12, "after_pt": 6 }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let document = String::from_utf8(zip_entries(&patched.bytes)["word/document.xml"].clone())
        .expect("utf8");
    assert!(document.contains(r#"w:before="240""#), "12pt = 240 twips: {document}");
    assert!(document.contains(r#"w:after="120""#), "6pt = 120 twips: {document}");
    assert_untouched_entries_identical(&before, &patched.bytes, &["word/document.xml"]);
}

#[test]
fn set_paragraph_spacing_rejects_both_omitted() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write");
    let error = DocxFormat
        .validate_edit(
            &DocxFormat.parse(&path).expect("parse"),
            &[SemanticOperation {
                kind: "set_paragraph_spacing".into(),
                payload: serde_json::json!({ "index": 1 }),
            }],
        )
        .expect_err("empty");
    assert!(error.to_string().contains("before_pt") || error.to_string().contains("spacing"));
}
```

`insert_page_break.rs`:

```rust
#[test]
fn insert_page_break_prepends_br() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    let before = fixture::minimal_docx();
    fs::write(&path, &before).expect("write");
    let handler = DocxFormat;
    let edit = handler
        .validate_edit(
            &handler.parse(&path).expect("parse"),
            &[SemanticOperation {
                kind: "insert_page_break".into(),
                payload: serde_json::json!({ "index": 1 }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let document = String::from_utf8(zip_entries(&patched.bytes)["word/document.xml"].clone())
        .expect("utf8");
    assert!(document.contains(r#"<w:br w:type="page"/>"#), "{document}");
    assert_untouched_entries_identical(&before, &patched.bytes, &["word/document.xml"]);
}
```

Duplicate zip helpers in each file.

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p dotall-docx --test set_paragraph_spacing --test insert_page_break`

Expected: FAIL.

- [ ] **Step 3: Implement**

`validate_set_paragraph_spacing`: resolve paragraph; parse optional `before_pt` / `after_pt` as finite `f64` ≥ 0; require at least one; twips = `(pt * 20.0).round() as u32`.

`patch_paragraph_spacing`: clone `patch_paragraph_alignment`; upsert `<w:spacing w:before="240" w:after="120"/>` inside `w:pPr` (preserve the other attr when only one is set: if spacing exists, merge attributes). Helper `upsert_p_spacing` analogous to `upsert_p_jc`.

`validate_insert_page_break`: editable body/table paragraph.

`patch_insert_page_break`: after `w:p` open (and after `w:pPr` if present), insert `<w:r><w:br w:type="page"/></w:r>`.

Capabilities + unsupported-kind list + skill. Only `word/document.xml` changes.

- [ ] **Step 4: Run tests**

Run: `cargo test -p dotall-docx --test set_paragraph_spacing --test insert_page_break && cargo test -p dotall-docx && cargo clippy -p dotall-docx -- -D warnings`

Expected: PASS.

- [ ] **Step 5: Controller commit**

```bash
git add crates/dotall-docx/src/format.rs crates/dotall-docx/src/edits.rs crates/dotall-docx/tests/set_paragraph_spacing.rs crates/dotall-docx/tests/insert_page_break.rs skills/docx/SKILL.md
git commit -m "$(cat <<'EOF'
feat(docx): set_paragraph_spacing and insert_page_break

EOF
)"
```

---

### Task 17: PDF `rotate_page`

**Files:**
- Create: `crates/dotall-pdf/tests/rotate_page.rs`
- Modify: `model.rs`, `parser.rs`, `format.rs`, `edits.rs`, `skills/pdf/SKILL.md`

**Interfaces:**
- Consumes: page dict `/Rotate`, `get_pages()`
- Produces: `PdfPageModel.rotate: Option<u32>` (`skip_serializing_if` 0/absent); inspect `pages[].rotate` when set; `rotate_page` payload `{ page, degrees }`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn rotate_page_sets_rotate_90() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, minimal_form_pdf()).expect("write");
    let handler = PdfFormat;
    let edit = handler
        .validate_edit(
            &handler.parse(&path).expect("parse"),
            &[SemanticOperation {
                kind: "rotate_page".into(),
                payload: serde_json::json!({ "page": 1, "degrees": 90 }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_pdf_bytes(&patched.bytes).expect("reparse");
    assert_eq!(after.pages[0].rotate, Some(90));
    let inspection = handler
        .inspect(&handler.parse({
            fs::write(&path, &patched.bytes).expect("write");
            &path
        }).expect("parse"))
        .expect("inspect");
    assert_eq!(inspection.summary["pages"][0]["rotate"], 90);
}

#[test]
fn rotate_page_rejects_45_degrees() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, minimal_form_pdf()).expect("write");
    let error = PdfFormat
        .validate_edit(
            &PdfFormat.parse(&path).expect("parse"),
            &[SemanticOperation {
                kind: "rotate_page".into(),
                payload: serde_json::json!({ "page": 1, "degrees": 45 }),
            }],
        )
        .expect_err("45");
    assert!(error.to_string().contains("90") || error.to_string().contains("degrees"));
}
```

Fix the inspect test to write then parse cleanly (avoid the inline `{ fs::write; &path }` if it is awkward):

```rust
    fs::write(&path, &patched.bytes).expect("rewrite");
    let inspection = handler
        .inspect(&handler.parse(&path).expect("parse"))
        .expect("inspect");
```

Also assert form field values unchanged (`Name` still `Ada` on `minimal_form_pdf`).

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p dotall-pdf --test rotate_page`

Expected: FAIL.

- [ ] **Step 3: Implement**

Parser: page `/Rotate` integer → `rotate: Some(v)` if v != 0. Inspect: include `pages: [{ number, rotate }]` **or** add `rotate` onto existing summary without dropping `page_count` / fields — locked: add `summary.pages` as `[{ "number", "rotate" }]` **only if** that does not break CLI smoke that keys `page_count`. Safer: put `rotate` on each object in a new `summary.page_rotations` array `{ "page": 1, "rotate": 90 }` and also on `PdfPageModel` for reparse tests. Locked: `summary.page_rotations` plus model field. Do not remove `page_count`.

Apply: `get_object_mut(page_id)` Dictionary `set("Rotate", Object::Integer(degrees as i64))`. `0` removes `/Rotate` if present. Do not rewrite content streams.

Capability + skill. Encrypted/signed still rejected at `validate` top.

- [ ] **Step 4: Run tests**

Run: `cargo test -p dotall-pdf --test rotate_page && cargo test -p dotall-pdf && cargo clippy -p dotall-pdf -- -D warnings`

Expected: PASS.

- [ ] **Step 5: Controller commit**

```bash
git add crates/dotall-pdf/src/model.rs crates/dotall-pdf/src/parser.rs crates/dotall-pdf/src/format.rs crates/dotall-pdf/src/edits.rs crates/dotall-pdf/tests/rotate_page.rs skills/pdf/SKILL.md
git commit -m "$(cat <<'EOF'
feat(pdf): rotate_page via /Rotate

EOF
)"
```

---

### Task 18: Wave 25 demo smoke (controller)

**Files:** `demo/README.md`, skills if needed. No binary regen required unless font/fill should be seeded (prefer CLI apply in README rather than baking styles into generate_demos).

- [ ] **Step 1:** README Wave 25 blocks:

```bash
# Wave 25: report styling
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"set_cell_font","payload":{"sheet":"Inputs","address":"A1","bold":true,"size_pt":14,"color":"#1F4E79"}}]'
$DOTALL apply demo/financials.xlsx --all
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"set_cell_fill","payload":{"sheet":"Inputs","address":"A1","color":"#D6DCE4"}}]'
$DOTALL apply demo/financials.xlsx --all

$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"set_table_cell_italic","payload":{"slide":"Slide 1","table":"Metrics","row":1,"col":1,"italic":true}}]'
$DOTALL apply demo/deck.pptx --all

$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"set_paragraph_spacing","payload":{"index":1,"before_pt":12,"after_pt":6}}]'
$DOTALL apply demo/memo.docx --all
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"insert_page_break","payload":{"index":3}}]'
$DOTALL apply demo/memo.docx --all

$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"rotate_page","payload":{"page":1,"degrees":90}}]'
$DOTALL apply demo/form.pdf --all
$DOTALL inspect demo/form.pdf
# expect page_rotations[0].rotate 90
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"rotate_page","payload":{"page":1,"degrees":0}}]'
$DOTALL apply demo/form.pdf --all
```

Then prior v0 smoke for all four formats.

- [ ] **Step 2:** `cargo test --workspace && cargo clippy --workspace -- -D warnings && cargo fmt --check`
- [ ] **Step 3:** Controller commit `docs(demo): Wave 25 report styling smoke`

---

## Standing demo gate (every wave)

After **each** of Waves 23, 23b (if demos changed), 24, 25:

1. Skills describe insert-only comments/pictures and chart read-only.
2. CLI smoke on **new** ops **and** prior v0 (`set_cell_value`, `set_shape_text`, `set_paragraph_text`, `set_form_field`).
3. `cargo test --workspace`, clippy `-D warnings`, `cargo fmt --check`.
4. Controller commit + push to `feat/office-parity-atlas`. **Do not merge.**

---

## Spec coverage self-check

| Spec requirement | Task |
|------------------|------|
| XLSX comment inspect + insert new; reject edit/delete/replace | 1 |
| PPTX comment inspect + insert new; reject mutate | 2 |
| DOCX comment inspect + insert new; reject in-place mutate | 3 |
| PDF text/sticky annot insert; no content-stream rewrite | 4 |
| Wave 23 skills + demo smoke | 5 |
| XLSX/PPTX/DOCX `charts[]` inspect; reject mutate; PDF omit | 6, 7, 8 |
| XLSX pictures inspect + insert; existing drawings identical | 9 |
| PPTX pictures inspect + insert; `ppt/media/*` identical | 10 |
| DOCX inline picture insert; `word/media/*` identical | 11 |
| PDF stamp annot; not content-stream | 12 |
| Wave 24 demo smoke | 13 |
| XLSX cell font + fill; `styles.xml` allowed | 14 |
| PPTX table cell italic (not fill) | 15 |
| DOCX spacing + page break | 16 |
| PDF `rotate_page` | 17 |
| Wave 25 demo smoke | 18 |
| Surgical OOXML / no unified IR / no formula engine / no chart insert | Global Constraints |
| Controller commits; branch; no merge | Global Constraints + each Step 5 |

## Placeholder scan

No TBD / TODO / “similar to Task N” without copied code. Zip helpers are repeated per test file (existing crate pattern).

## Type/name consistency

- Ops: `insert_comment`, `insert_picture`, `set_cell_font`, `set_cell_fill`, `set_table_cell_italic`, `set_paragraph_spacing`, `insert_page_break`, `rotate_page`
- Models: `CommentModel`, `PictureModel`, `ChartModel` (format-owned fields)
- Id prefixes: `cm_`, `pic_`, `ch_`
- SCHEMA_VERSION remains `1`
- XLSX comments = Notes (`Note` + `insert_note`), not threaded comments
