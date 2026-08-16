# Office + PDF Format Families Design

**Date:** 2026-08-15  
**Status:** Approved for planning  
**Related:**
- `docs/specs/core-format-architecture.md`
- `docs/specs/dotall-overview.md`
- `docs/superpowers/specs/2026-08-12-snapshot-delta-storage-design.md`
- `docs/superpowers/specs/2026-07-21-mcp-agent-interface-design.md`

## Goal

Add **PPTX**, **DOCX**, and **PDF** as independent format-family crates behind the
existing `FormatHandler` contract: agent-readable models, token-budgeted reads,
capability-discovered semantic edits, surgical (or PDF-safe) apply, and lossless
history/revert. Do **not** invent a universal Office AST.

## Decisions

| Topic | Choice |
|-------|--------|
| Order | Shared OOXML ZIP snapshots → **PPTX** → **DOCX** → **PDF** |
| Crate layout | `dotall-pptx`, `dotall-docx`, `dotall-pdf`; optional `dotall-ooxml` for ZIP part encode/decode only |
| Registration | Cargo features on CLI/MCP (`pptx`, `docx`, `pdf`); default keeps `xlsx` and adds each as it ships |
| Snapshots (OOXML) | Same part+manifest grain as XLSX (`ooxml.snapshot-manifest` v1, `format_id` on envelope) |
| Snapshots (PDF) | Opaque full-blob default; incremental PDF updates are a follow-up |
| Intra-part XML deltas | **Out of v0** (follow-up after DOCX body-history evidence) |
| Agent surface | Unchanged MCP tools; discover ops via `dotall_capabilities` |
| CLI reads | Add generic `--selector-kind` / `--selector`; keep XLSX convenience flags |
| Detection | Extension **and** package evidence; generic ZIP must not steal PPTX/DOCX from XLSX |

## Why this order

1. **Shared OOXML snapshots** — XLSX already explodes ZIP slices; PPTX/DOCX must not
   copy `snapshot.rs`. Also **fix XLSX ZIP detection** (today any `PK` prefix scores 60).
2. **PPTX** — slides/media isolate like sheets; surgical edits and part savings are
   the closest twin of XLSX.
3. **DOCX** — high demand; body lives in one `document.xml`, so part-level history
   is weaker; still the right Word slice (read + paragraph text patch).
4. **PDF** — not OOXML. v0 is structured read + form fill / annotations, not
   “edit this paragraph in a designed page.”

## Shared core (no change to Engine)

Each crate implements:

```text
detect → parse / parse_bytes → inspect → read
validate_edit[_with_source] → apply_edit_bytes
encode_snapshot_with_hash / decode_snapshot
```

`Engine` stage/apply/history/revert stays format-agnostic. `snapshot_ref` remains
blake3 of reconstructed full file bytes.

## Format v0 scopes

### PPTX (`pptx.presentation` v1)

**Model:** presentation → slides → shapes with text frames (runs concatenated) +
notes; media listed as unmodeled preserve-only. Stable `element_id`s on slides and
shapes.

**Read selectors:** `full`, `slide` (`Slide 3` or `slide3`), `notes`.

**Edits:** `set_shape_text` (required); `replace_slide_text` optional if one-op
replace is cheaper for agents. Reject SmartArt / charts / animations.

**Write:** patch `ppt/slides/slideN.xml` only (and that slide’s `.rels` only if
required). Masters, theme, other slides, `ppt/media/*` byte-identical.

**Snapshots:** OOXML part encode.

### DOCX (`docx.document` v1)

**Model:** body paragraphs (index + outline level if heading style) → runs’
concatenated text; tables as paragraph-like cells later. Headers/footers/comments
listed in inspect, not edited in v0.

**Read selectors:** `full` (budgeted), `paragraphs` (`1:40` or heading name).

**Edits:** `set_paragraph_text` replacing all `w:t` in that `w:p` while preserving
the first run’s `w:rPr` when possible. Reject tracked-changes / content-controls /
complex fields when present on the target para.

**Write:** patch `word/document.xml` only for v0 body edits.

**Snapshots:** OOXML part encode (honest: body edits rewrite the large part).

### PDF (`pdf.document` v1)

**Model:** page count, per-page extracted text (best-effort), outline titles,
AcroForm fields (`name`, type, current value, page).

**Read selectors:** `full` (outline + field map + first pages), `page` (`3`),
`field` (`CustomerName`).

**Edits:** `set_form_field` only. No body-text rewrite, no page delete/rotate in v0.

**Write:** `lopdf` form-value update; preserve other objects. If a file is
encrypted or certified-signed, reject.

**Snapshots:** opaque blob (FormatHandler default).

## Non-goals (this wave)

- Intra-`document.xml` / intra-slide XML deltas
- PPTX slide add/delete/reorder (follow-up; touches `presentation.xml` + rels)
- DOCX styles, numbering, comments, track-changes apply
- PDF incremental xref updates, annotations, page surgery, digital-signature
  preservation beyond reject-if-signed
- Universal document graph
- Python/JS SDKs

## Testing bar (all three)

- Detect: correct winner among `.xlsx`/`.pptx`/`.docx`/`.pdf` sharing ZIP or
  similar prefixes
- Inspect/read: golden projection on a fixture
- Edit: semantic result correct; **untouched ZIP entries byte-identical** (Office)
- Snapshot: `decode(encode(bytes)) == bytes`; PPTX media reuse across slide-text
  applies
- CLI + MCP register via features; `skills/<format>/SKILL.md`
- `cargo test --workspace`, `cargo fmt`, clippy `-D warnings`

## Follow-ups

- Extract more OOXML writer helpers only after PPTX + DOCX share a second pattern
- DOCX inner-part deltas once body history size is measured
- PDF incremental updates / annotations
- PPTX structural slide ops
