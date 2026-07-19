# XLSX Read Pipeline + Translation AST Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship cached XLSX detect → parse → inspect/read with a sturdy translation AST (canonical model, lightweight structure derived, dual agent dialects) on top of the existing project-local store.

**Architecture:** Core owns object-safe format contracts, envelopes, artifact cache, budgets, and `Engine` orchestration. `dotall-xlsx` owns `xlsx.workbook` v1, hybrid IDs, detection, structure derived, and Markdown/`ast_range` projections. CLI composes the registry. Follow `docs/superpowers/specs/2026-07-19-xlsx-translation-ast-design.md` exactly for model/projection shape.

**Tech Stack:** Stable Rust workspace, calamine, serde/serde_json, blake3, clap, rust_xlsxwriter (dev fixtures), existing `DotallStore`.

**Spec authority:** Translation AST design supersedes the thin cell-dump in the older `2026-07-18-xlsx-read-pipeline.md` plan. Prefer the design doc when they conflict.

**Commit policy:** User authorized plan execution including per-task commits on `feat/xlsx-read-pipeline`.

---

## Target file structure

```text
crates/
├── dotall-core/src/
│   ├── registry/{mod.rs,types.rs}
│   ├── pipeline/{mod.rs,artifact.rs}
│   ├── read/{mod.rs,budget.rs}
│   ├── orchestrate/mod.rs
│   ├── store/mod.rs          # + model/view/derived cache helpers
│   ├── error.rs              # + format/cache errors
│   └── lib.rs
├── dotall-xlsx/src/
│   ├── lib.rs
│   ├── model.rs              # xlsx.workbook v1 types + ID helpers
│   ├── parser.rs
│   ├── ids.rs                # deterministic hybrid ID generation
│   ├── detection.rs
│   ├── structure.rs          # lightweight xlsx.structure derived
│   ├── selector.rs
│   ├── projection.rs         # inspect + Markdown + ast_range
│   └── format.rs             # FormatHandler impl
└── dotall-cli/src/main.rs    # inspect + read commands
```

---

### Task 1: Finish feature-gated XLSX crate

**Files:**
- Ensure: `crates/dotall-xlsx/` (may already exist uncommitted)
- Modify: `Cargo.toml`, `crates/dotall-cli/Cargo.toml`

- [ ] **Step 1: Verify crate wiring**

Confirm workspace members include `crates/dotall-xlsx`, CLI has:

```toml
[features]
default = ["xlsx"]
xlsx = ["dep:dotall-xlsx"]
```

and `dotall-xlsx` depends on `dotall-core`, `serde`, `serde_json`, `calamine`, with dev-deps `tempfile`, `rust_xlsxwriter`.

- [ ] **Step 2: Ensure smoke test**

`crates/dotall-xlsx/src/lib.rs` exports `FORMAT_ID = "xlsx"` and a unit test asserting it.

- [ ] **Step 3: Verify**

```bash
cargo test -p dotall-xlsx
cargo check -p dotall-cli --no-default-features
cargo check -p dotall-cli
```

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock crates/dotall-xlsx crates/dotall-cli/Cargo.toml
git commit -m "build: add feature-gated XLSX format crate"
```

---

### Task 2: Format registry and read contracts

**Files:**
- Create: `crates/dotall-core/src/registry/types.rs`
- Create: `crates/dotall-core/src/registry/mod.rs`
- Create: `crates/dotall-core/src/read/mod.rs`
- Create: `crates/dotall-core/src/read/budget.rs` (stub `apply_budget` returning full content)
- Modify: `crates/dotall-core/src/error.rs`
- Modify: `crates/dotall-core/src/lib.rs`

- [ ] **Step 1: Write registry selection test** in `registry/mod.rs`

Highest `DetectionScore` wins; zero scores are ignored; no match → `UnsupportedFormat`.

- [ ] **Step 2: Define types** in `registry/types.rs`

Include: `DetectionScore`, `DetectionProbe`, `FormatDescriptor`, `Capability` (at least `Inspect`, `ReadFull`, `ReadSelector { kind }`), `ArtifactEnvelope` (`format_id`, `schema_id`, `schema_version`, `payload`), `Inspection` (`format_id`, `summary`, `capabilities`, `suggested_reads`), `ReadSuggestion`, `ReadSelector`, `ReadRequest` (`selector`, `max_tokens`, `continuation`), `ReadResponse` (`content`, `estimated_tokens`, `truncated`, `continuation`, `next_actions`).

- [ ] **Step 3: Define `FormatHandler`**

```rust
pub trait FormatHandler: Send + Sync {
    fn descriptor(&self) -> FormatDescriptor;
    fn detect(&self, probe: &DetectionProbe<'_>) -> DetectionScore;
    fn parse(&self, source: &Path) -> Result<ArtifactEnvelope>;
    fn inspect(&self, model: &ArtifactEnvelope) -> Result<Inspection>;
    fn read(&self, model: &ArtifactEnvelope, request: &ReadRequest) -> Result<ReadResponse>;
}
```

No edit methods in this plan (YAGNI; land with edit plan). Document that in a short module comment.

- [ ] **Step 4: Add errors**

`UnsupportedFormat`, `ArtifactSchemaMismatch`, `UnsupportedCapability`, `Format { format_id, path, message }`, `Serialization { context, source }`, and prefer these (not `InvalidManifest`) for non-manifest JSON decode failures in later tasks.

- [ ] **Step 5: Export modules and run** `cargo test -p dotall-core registry`

- [ ] **Step 6: Commit** `feat(core): define format and read contracts`

---

### Task 3: Persist model, derived, and view artifacts

**Files:**
- Create: `crates/dotall-core/src/pipeline/{mod.rs,artifact.rs}`
- Modify: `crates/dotall-core/src/store/mod.rs`
- Create: `crates/dotall-core/tests/artifact_cache.rs`
- Modify: `crates/dotall-core/src/lib.rs`

- [ ] **Step 1: Failing integration test**

Register a source, `write_model` / `read_model` round-trip `ArtifactEnvelope`. Assert `read_model` returns `None` when cached `source_hash` mismatches tracked fingerprint.

Also round-trip `CachedView` and a `CachedDerived` under `cache/derived/<name>.json`.

- [ ] **Step 2: Implement** `CachedArtifact`, `CachedView`, `CachedDerived` with `source_hash`, producer/processor identity fields, and payload/response as in the design.

- [ ] **Step 3: Store methods**

`write_model` / `read_model`, `write_view` / `read_view`, `write_derived` / `read_derived`. Decode errors use `Serialization` or a dedicated cache error — not `InvalidManifest`.

- [ ] **Step 4: Verify** `cargo test -p dotall-core --test artifact_cache`

- [ ] **Step 5: Commit** `feat(core): persist source-bound model, derived, and view artifacts`

---

### Task 4: Canonical `xlsx.workbook` v1 model + deterministic IDs

**Files:**
- Create: `crates/dotall-xlsx/src/model.rs`
- Create: `crates/dotall-xlsx/src/ids.rs`
- Modify: `crates/dotall-xlsx/src/lib.rs`

- [ ] **Step 1: Unit tests for IDs**

Same sheet name + address + schema version → same `element_id`. Different address → different id. IDs look opaque (`c_…` / `sh_…` / `wb_…`) not raw `sheet:Name/cell:A1`.

- [ ] **Step 2: Implement types** matching the design doc:

`WorkbookModel`, `SheetModel`, `CellModel`, `CellValue`, `NamedRange`, `StyleEntry` (minimal), `UnmodeledMap`, `column_name` helper.

Constants: `SCHEMA_ID = "xlsx.workbook"`, `SCHEMA_VERSION = 1`.

- [ ] **Step 3: Verify** `cargo test -p dotall-xlsx`

- [ ] **Step 4: Commit** `feat(xlsx): define workbook v1 model and hybrid IDs`

---

### Task 5: Parse workbooks into the canonical model

**Files:**
- Create: `crates/dotall-xlsx/src/parser.rs`
- Create: `crates/dotall-xlsx/tests/read_contract.rs`
- Modify: `crates/dotall-xlsx/src/lib.rs`

- [ ] **Step 1: Failing fixture test** with `rust_xlsxwriter`

Assert sheet name, sparse cells, formula on a cell, hybrid `element_id` + `address`, deterministic re-parse IDs.

- [ ] **Step 2: Implement `parse_workbook`** via calamine

Populate sheets, sparse cells, formulas, merges if available from calamine (or empty merges with a comment if API lacking), named ranges if available else empty, minimal `style_table` / `number_format` when available else empty/None, `unmodeled` preserve map always set.

Map calamine errors to `DotallError::Format`.

- [ ] **Step 3: Verify** `cargo test -p dotall-xlsx --test read_contract`

- [ ] **Step 4: Commit** `feat(xlsx): parse workbooks into xlsx.workbook v1`

---

### Task 6: Detection, selectors, structure derived, projections, FormatHandler

**Files:**
- Create: `crates/dotall-xlsx/src/{detection,selector,structure,projection,format}.rs`
- Modify: `crates/dotall-xlsx/src/lib.rs`
- Modify: `crates/dotall-xlsx/tests/read_contract.rs`

- [ ] **Step 1: Tests**

- ZIP magic + `.xlsx` extension → high detection score
- `inspect` summary includes sheet dims, formula count, `preserved`, suggested `range` and `ast_range`
- Markdown `range` read includes values and a drill-down hint
- `ast_range` returns JSON containing `element_id` and `address` for cells
- Lightweight structure: header_row heuristic on first row of used range (confidence + warnings OK)

- [ ] **Step 2: Implement modules**

Selectors: `sheet`, `range`, `full`, `ast_range` (parse `Sheet!A1:D20`).  
Projections per design dialects.  
`XlsxFormat` implements `FormatHandler`; `parse` wraps model in `ArtifactEnvelope` with schema id/version; decode rejects mismatches via `ArtifactSchemaMismatch`.

- [ ] **Step 3: Verify** `cargo test -p dotall-xlsx`

- [ ] **Step 4: Commit** `feat(xlsx): detect, project, and expose FormatHandler`

---

### Task 7: Token budgets and Engine orchestration

**Files:**
- Replace stub: `crates/dotall-core/src/read/budget.rs`
- Create: `crates/dotall-core/src/orchestrate/mod.rs`
- Create: `crates/dotall-core/tests/read_engine.rs`
- Modify: `crates/dotall-core/src/lib.rs`
- Modify: store if needed for access log helper

- [ ] **Step 1: Budget tests**

Truncation sets `truncated=true`, continuation cursor, resume concatenates.

Simple estimator OK (e.g. chars/4) if documented.

- [ ] **Step 2: Engine cache tests** with counting stub handler

- Second `inspect` does not re-parse (parse count == 1)
- Overwriting source content increments parse count on next inspect
- Engine always checks live freshness before `read_model`

- [ ] **Step 3: Implement `Engine`**

Flow from design §6: detect → auto-register → freshness → model cache → optional structure derived (call format helper or skip if handler lacks derive; for v0, structure may be computed inside `inspect` from model without a separate FormatHandler method — prefer a `dotall_xlsx::structure::derive` invoked from Engine only when handler is xlsx is wrong; keep structure inside `XlsxFormat::inspect` for this plan to avoid core→xlsx).  

**Decision for this plan:** `inspect`/`read` on the handler produce projections; Engine only caches model + views. Structure enrichment happens inside `XlsxFormat::inspect` (may embed structure hints in summary). Separate `write_derived` for structure can be called from xlsx helper used by Engine via an optional `FormatHandler::ensure_derived` **or** simply computed ephemerally in inspect for v0. Prefer: add optional default method later; for v0 compute structure inside xlsx inspect and optionally persist via a method on `XlsxFormat` that Engine does not need if inspect is self-contained.

Keep Engine format-agnostic: only `parse`/`inspect`/`read` + model/view cache.

Append access log lines to `state/access/log.jsonl` after successful inspect/read.

- [ ] **Step 4: Verify** `cargo test -p dotall-core --test read_engine` and budget unit tests

- [ ] **Step 5: Commit** `feat(core): orchestrate cached inspect and read`

---

### Task 8: CLI `inspect` and `read`

**Files:**
- Modify: `crates/dotall-cli/src/main.rs`
- Create: `crates/dotall-cli/tests/xlsx_read.rs`

- [ ] **Step 1: CLI tests**

Init workspace, write fixture xlsx, `dotall inspect path`, `dotall read path --range 'Sheet!A1:B2'`, warm inspect does not rewrite model mtime, `--json` works, missing xlsx feature path errors clearly when built `--no-default-features` (optional check).

- [ ] **Step 2: Implement commands** composing registry with `XlsxFormat` when feature enabled.

- [ ] **Step 3: Verify** `cargo test -p dotall-cli`

- [ ] **Step 4: Commit** `feat(cli): inspect and read XLSX workbooks`

---

### Task 9: Quality gate

- [ ] **Step 1: Run**

```bash
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo check -p dotall-cli --no-default-features
```

- [ ] **Step 2: Fix failures**

- [ ] **Step 3: Commit** only if fixes needed: `chore: read-pipeline quality gate`

---

## Self-review

- Matches translation AST design: layered artifacts, hybrid IDs, dual dialects, B fidelity, preserve-only unmodeled.
- Formula dependency graph deferred.
- Edit/validate_edit deferred.
- Core never depends on `dotall-xlsx`.
