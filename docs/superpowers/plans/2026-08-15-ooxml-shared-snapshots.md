# OOXML Shared Snapshots + Detection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract ZIP part snapshot encode/decode from `dotall-xlsx` into `dotall-ooxml`, and stop XLSX from claiming every ZIP (so PPTX/DOCX can detect correctly).

**Architecture:** `dotall-ooxml` is a tiny crate: slice-based package explode/assemble plus ZIP entry-name probes for detection. `XlsxFormat` calls it with `format_id = "xlsx"`. Engine and store APIs do not change.

**Tech Stack:** Existing `zip` crate, blake3, serde_json `EncodedSnapshot`.

## Global Constraints

- `decode(encode(bytes))` remains byte-identical for XLSX fixtures already covered by `crates/dotall-xlsx/src/snapshot.rs` tests
- Manifest `schema_id` stays `xlsx.snapshot-manifest` for XLSX (pass format-specific schema id into the helper)
- Core never depends on `dotall-ooxml`
- No PPTX/DOCX crates in this plan
- Generic ZIP without `xl/workbook.xml` must not score as a strong XLSX match

---

## File structure

- Create: `crates/dotall-ooxml/Cargo.toml`
- Create: `crates/dotall-ooxml/src/lib.rs`
- Create: `crates/dotall-ooxml/src/snapshot.rs` (move from xlsx)
- Create: `crates/dotall-ooxml/src/probe.rs`
- Modify: `Cargo.toml` (workspace members)
- Modify: `crates/dotall-xlsx/Cargo.toml` (depend on `dotall-ooxml`)
- Modify: `crates/dotall-xlsx/src/snapshot.rs` (thin wrapper or delete)
- Modify: `crates/dotall-xlsx/src/format.rs` (encode/decode via helper)
- Modify: `crates/dotall-xlsx/src/detection.rs`
- Test: `crates/dotall-xlsx/src/snapshot.rs` existing tests; new detection tests
- Test: `crates/dotall-ooxml/src/lib.rs` unit tests for probe

---

### Task 1: Create `dotall-ooxml` and move snapshot encode/decode

**Files:**
- Create: `crates/dotall-ooxml/Cargo.toml`
- Create: `crates/dotall-ooxml/src/lib.rs`
- Create: `crates/dotall-ooxml/src/snapshot.rs`
- Modify: `/Users/harshitmorj/Documents/dotall-dev/dotall-core/Cargo.toml`
- Modify: `crates/dotall-xlsx/Cargo.toml`
- Modify: `crates/dotall-xlsx/src/snapshot.rs`
- Modify: `crates/dotall-xlsx/src/format.rs`

**Interfaces:**
- Consumes: current `dotall-xlsx` slice encoder (`encode(package, package_hash: Option<&str>)`)
- Produces:

```rust
pub fn encode_package(
    package: &[u8],
    format_id: &str,
    manifest_schema_id: &str,
    package_hash: Option<&str>,
) -> dotall_core::Result<dotall_core::EncodedSnapshot>;

pub fn decode_package(
    encoded: &dotall_core::EncodedSnapshot,
) -> dotall_core::Result<Vec<u8>>;
```

- [ ] **Step 1: Add the crate to the workspace**

`Cargo.toml` members: add `"crates/dotall-ooxml"`.

`crates/dotall-ooxml/Cargo.toml`:

```toml
[package]
name = "dotall-ooxml"
version = "0.1.0"
edition = "2024"

[dependencies]
blake3 = "1.8.5"
dotall-core = { version = "0.1.0", path = "../dotall-core" }
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.150"
zip = { version = "8.6.0", default-features = false, features = ["deflate-flate2"] }
```

- [ ] **Step 2: Move `crates/dotall-xlsx/src/snapshot.rs` into `dotall-ooxml`**

Keep the slice-based algorithm (local header / payload / trailer + central-directory tail). Parameterize errors and manifest:

- `schema_id` argument instead of `xlsx.snapshot-manifest` constant
- `format_id` argument on `EncodedSnapshot.format_id`
- Error `format_id` in `DotallError::Format` uses the caller’s id

Public `lib.rs`:

```rust
mod probe;
mod snapshot;

pub use probe::{has_zip_magic, zip_contains_entry};
pub use snapshot::{decode_package, encode_package};
```

- [ ] **Step 3: Point XLSX at the helper**

`crates/dotall-xlsx/Cargo.toml` add `dotall-ooxml = { version = "0.1.0", path = "../dotall-ooxml" }`.

`XlsxFormat::encode_snapshot_with_hash`:

```rust
dotall_ooxml::encode_package(
    source_bytes,
    crate::FORMAT_ID,
    "xlsx.snapshot-manifest",
    Some(package_hash),
)
```

`decode_snapshot`: `dotall_ooxml::decode_package(encoded)`.

Delete duplicated encode/decode from xlsx, keep xlsx unit tests that call the public handler (move the private `package()` fixture tests to `dotall-ooxml` or keep them calling `encode_package`).

- [ ] **Step 4: Run XLSX snapshot tests**

Run: `cargo test -p dotall-xlsx --lib snapshot -- --nocapture`
and `cargo test -p dotall-xlsx --test snapshot_parts`

Expected: PASS (lossless reconstruct + part sharing).

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/dotall-ooxml crates/dotall-xlsx
git commit -m "$(cat <<'EOF'
extract OOXML ZIP part snapshot encode/decode into dotall-ooxml

EOF
)"
```

---

### Task 2: ZIP entry probe + fix XLSX detection

**Files:**
- Create: `crates/dotall-ooxml/src/probe.rs`
- Modify: `crates/dotall-xlsx/src/detection.rs`
- Test: `crates/dotall-xlsx/src/detection.rs` or `crates/dotall-xlsx/src/lib.rs` tests
- Modify: `crates/dotall-xlsx/src/format.rs` if `detect` needs more than prefix (see step 2)

**Interfaces:**
- Produces:

```rust
pub fn has_zip_magic(prefix: &[u8]) -> bool;
/// Returns true when the ZIP central directory lists `name` (forward slashes).
pub fn zip_contains_entry(package: &[u8], name: &str) -> bool;
```

- [ ] **Step 1: Write failing detection tests**

XLSX `detect` currently scores 60 for any ZIP. Add tests that build tiny zips (reuse xlsx snapshot test `ZipWriter` helper or `zip` crate):

```rust
#[test]
fn pptx_zip_is_not_a_strong_xlsx_match() {
    let bytes = zip_with_entry("ppt/presentation.xml", b"<p/>");
    let score = detection::score(&DetectionProbe {
        path: Path::new("deck.pptx"),
        prefix: &bytes[..bytes.len().min(16)],
    });
    // After the fix, extension-only pptx must not be 100, and
    // detect() that only sees prefix cannot claim workbook.xml.
    assert!(score.0 < 80, "pptx must not outrank a real xlsx handler");
}
```

Also add a handler-level test once `detect` can see full bytes **or** document that prefix-only detect stays weak (40 for `.xlsx` without magic, 0 for `.pptx`).

**Important:** `DetectionProbe` today is `path + prefix` only. Do **not** expand the probe in this task unless existing `Engine` already reads enough prefix. Check `read_prefix` in `orchestrate/mod.rs` (it reads a small prefix).

If prefix is too small to contain central-directory names, detection **must** use extension as the discriminator among OOXML families:

| evidence | score |
|----------|------:|
| `.xlsx` + ZIP magic | 100 |
| `.xlsx` without ZIP magic | 40 |
| ZIP magic + not `.xlsx` | **0** (was 60) |
| else | 0 |

PPTX/DOCX will score 100 on their own extensions in later plans. A ZIP named `file.bin` matching PK will no longer be “probably xlsx.”

- [ ] **Step 2: Run the new test to see current 60 score fail the assertion**

Run: `cargo test -p dotall-xlsx --lib pptx_zip -- --nocapture`  
Expected: FAIL (score 60 or 40 depending on extension).

- [ ] **Step 3: Implement probe + detection table**

`has_zip_magic` copies the three PK signatures from current `detection.rs`.

Change `detection::score` to the table above (drop “ZIP without xlsx extension → 60”).

Optional: `zip_contains_entry` implemented via `ZipArchive` for later PPTX/DOCX detect if Engine later passes more bytes; export it now, use in PPTX plan. Unit-test it in `dotall-ooxml` with a fixture zip.

- [ ] **Step 4: Run tests**

Run: `cargo test -p dotall-xlsx -p dotall-ooxml`  
Expected: PASS. Existing XLSX CLI/MCP inspect tests still pass.

- [ ] **Step 5: Commit**

```bash
git add crates/dotall-ooxml crates/dotall-xlsx
git commit -m "$(cat <<'EOF'
stop treating every ZIP as XLSX so PPTX and DOCX can detect

EOF
)"
```

---

### Task 3: Workspace gate + AGENTS pointer

**Files:**
- Modify: `AGENTS.md` (crate list: mention `dotall-ooxml` as ZIP snapshot helper, not a format)
- Modify: `docs/specs/core-format-architecture.md` only if the layout snippet lists crates (add `dotall-ooxml`)

- [ ] **Step 1: Run** `cargo test --workspace` and `cargo clippy --workspace -- -D warnings`
- [ ] **Step 2: Commit docs** if the architecture crate list is updated

```bash
git add AGENTS.md docs/specs/core-format-architecture.md
git commit -m "$(cat <<'EOF'
document dotall-ooxml as the shared ZIP snapshot helper

EOF
)"
```

---

## Spec coverage

- Extract shared ZIP encode/decode: Task 1
- Detection disambiguation: Task 2
- No PPTX/DOCX/PDF implementation in this plan (roadmap items 2–4)
