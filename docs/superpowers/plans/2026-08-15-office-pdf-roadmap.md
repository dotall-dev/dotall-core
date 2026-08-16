# Office + PDF Format Families Roadmap

> **For agentic workers:** Execute each linked plan with superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Reconcile later plans with actual APIs after each completed slice.

**Goal:** Ship PPTX, DOCX, and PDF as independent format crates on the existing Engine/MCP spine, with surgical (or PDF-safe) edits and lossless history.

**Architecture:** Core stays format-agnostic. A tiny `dotall-ooxml` crate owns ZIP part snapshot encode/decode used by XLSX then PPTX/DOCX. Each family crate owns model, parse, read, validate, and apply. CLI/MCP register handlers behind Cargo features.

**Tech Stack:** Stable Rust workspace, existing `FormatHandler`, `zip` + `quick-xml` for Office, `lopdf` for PDF, blake3 snapshots, serde JSON models.

**Design:** [`docs/superpowers/specs/2026-08-15-office-pdf-format-families-design.md`](../specs/2026-08-15-office-pdf-format-families-design.md)

## Execution order

1. [`2026-08-15-ooxml-shared-snapshots.md`](2026-08-15-ooxml-shared-snapshots.md)
   - Extract ZIP slice encode/decode from `dotall-xlsx`.
   - Disambiguate OOXML detection (XLSX must not win on every `PK` ZIP).
   - Exit gate: XLSX snapshot tests still lossless; `.pptx`/`.docx` fixtures do not detect as `xlsx` at score 100.

2. [`2026-08-15-pptx-engine-v0.md`](2026-08-15-pptx-engine-v0.md)
   - `dotall-pptx`: slides/shapes model, read, `set_shape_text`, part snapshots, skill.
   - Exit gate: edit one shape; other slides + `ppt/media/*` byte-identical; revert works.

3. [`2026-08-15-docx-engine-v0.md`](2026-08-15-docx-engine-v0.md)
   - `dotall-docx`: paragraphs model, read, `set_paragraph_text`, part snapshots, skill.
   - Exit gate: body text edit; headers/media byte-identical; tracked-changes target rejected.

4. [`2026-08-15-pdf-engine-v0.md`](2026-08-15-pdf-engine-v0.md)
   - `dotall-pdf`: pages/fields model, read, `set_form_field`, opaque snapshots, skill.
   - Exit gate: fill a text field; encrypted/signed files rejected; history revert restores bytes.

## Revision checkpoints

After every plan:

1. Run that plan’s quality gate (`cargo test --workspace`, fmt, clippy `-D warnings`).
2. Update CLI/MCP feature matrices and `AGENTS.md` crate list.
3. Do not extract more shared OOXML writer code until PPTX and DOCX both exist and share a second pattern.
4. Keep intra-part XML deltas, PDF incremental xref, and slide add/delete **out** of these v0 plans.

## Completion definition

This wave is complete when an agent can `dotall_init` → `capabilities` → `read` → `edit` → `apply` → `history`/`revert` on `.pptx`, `.docx`, and `.pdf` fixtures without unzipping packages or writing Python, and XLSX behavior is unchanged.
