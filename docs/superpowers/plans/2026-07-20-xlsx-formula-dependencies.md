# XLSX Formula Dependencies Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Parse Excel formula references (no evaluation), persist a source-bound dependency graph as a derived artifact, and expose forward/reverse agent queries — Phase 0 before staged writes.

**Architecture:** Extend existing `CachedDerived` / `write_derived` / `read_derived` with deterministic `DerivationRecipe` keys. Keep lexing, graph types, and build inside `dotall-xlsx`. Engine (or format helper) ensures the derived graph on demand; CLI adds dependency query commands. Aligns with `docs/superpowers/specs/2026-07-20-xlsx-write-history-design.md` and the translation AST design.

**Tech Stack:** Stable Rust, existing store/Engine, `blake3`, `serde`/`serde_json`. No formula evaluation.

**Spec authority:** Write/history design §9 + this plan. Older `2026-07-18-xlsx-formula-dependencies.md` is superseded where they conflict (especially store API: we already have name-based derived cache — extend it rather than replace blindly).

**Commit policy:** Per-task commits authorized when executing this plan on an implementation branch.

---

## Preconditions

- `main` includes storage foundation + XLSX read pipeline.
- Reconcile with: `CachedDerived`, `DotallStore::write_derived`/`read_derived`, `FormatHandler`, `Engine`, `xlsx.workbook` v1 model.

## Scope

**In:** A1 refs/`$`, ranges, sheet-qualified refs, named ranges present in model, string-literal skipping, forward/reverse queries, cache hit on identical recipe.

**Out:** Evaluation, expanding ranges to every cell, external workbooks, formula rewrite, edit/apply.

## Target files

```text
crates/dotall-core/src/pipeline/{recipe.rs,artifact.rs,mod.rs}
crates/dotall-core/src/store/mod.rs
crates/dotall-core/src/orchestrate/mod.rs          # ensure_derived helper path
crates/dotall-core/tests/derived_cache.rs
crates/dotall-xlsx/src/dependencies/{mod.rs,lexer.rs,graph.rs}
crates/dotall-xlsx/src/{format.rs,projection.rs,lib.rs}
crates/dotall-xlsx/tests/dependencies.rs
crates/dotall-cli/src/main.rs
crates/dotall-cli/tests/xlsx_dependencies.rs
```

---

### Task 1: Derivation recipes

**Files:** Create `pipeline/recipe.rs`; modify `pipeline/mod.rs`, `artifact.rs` if needed.

- [ ] **Step 1: Failing tests** — `DerivationRecipe` key deterministic; changes when any field changes.

```rust
pub struct DerivationRecipe {
    pub source_hash: String,
    pub processor_id: String,
    pub processor_version: String,
    pub config_hash: String,
    pub input_hashes: Vec<String>,
}
impl DerivationRecipe {
    pub fn key(&self) -> Result<String> { /* blake3(serde_json) */ }
}
```

- [ ] **Step 2: Implement + export**

- [ ] **Step 3:** `cargo test -p dotall-core pipeline::recipe`

- [ ] **Step 4: Commit** `feat(core): define deterministic derivation recipes`

---

### Task 2: Recipe-keyed derived cache on store

**Files:** Modify `store/mod.rs`; create `tests/derived_cache.rs`.

Existing API writes `cache/derived/<name>.json`. Extend so formula deps use **recipe key** as the file name (or `formula-dependencies-<key>.json`), and `read_derived` returns `None` when embedded recipe/source/processor fields mismatch.

- [ ] **Step 1: Failing test** — write with recipe A hits; recipe with different `config_hash` misses; source_hash mismatch misses; processor_version mismatch misses.

- [ ] **Step 2: Implement** helpers e.g. `write_derived_recipe` / `read_derived_recipe` **or** overload name=`recipe.key()` and validate `CachedDerived` fields on read. Prefer keeping `write_derived`/`read_derived` and documenting that callers pass `recipe.key()` as `name`, while validating processor/schema/source on read (extend validation beyond source_hash alone for derived).

- [ ] **Step 3:** `cargo test -p dotall-core --test derived_cache`

- [ ] **Step 4: Commit** `feat(core): validate recipe-bound derived artifact cache`

---

### Task 3: Formula reference lexer

**Files:** Create `dotall-xlsx/src/dependencies/lexer.rs`, `mod.rs`.

- [ ] **Step 1: Tests** — `A1`, `$B$12`, `A1:B2`, `Sheet1!A1`, `'My Sheet'!A1:B2`, named range token; ignore refs inside `"..."` strings; ignore non-refs.

- [ ] **Step 2: Implement** lexer returning structured ref tokens (sheet?, start, end?, absolute flags).

- [ ] **Step 3:** `cargo test -p dotall-xlsx dependencies::lexer`

- [ ] **Step 4: Commit** `feat(xlsx): lex formula references without evaluating`

---

### Task 4: Dependency graph build

**Files:** Create `dependencies/graph.rs`; tests in `tests/dependencies.rs`.

```rust
pub struct DependencyGraph {
    pub edges: Vec<DependencyEdge>, // from_element_id -> to_element_id / selector
}
```

- [ ] **Step 1: Fixture workbook** with formulas across sheets + named range; assert forward deps of sink cell and reverse deps of source cell. Ranges as single edge to range selector (do **not** explode to all cells).

- [ ] **Step 2: `build(model: &WorkbookModel) -> DependencyGraph`** using lexer + named_ranges map; stable element_ids from model.

- [ ] **Step 3:** Envelope helper `to_artifact(graph) -> ArtifactEnvelope` with `schema_id = "xlsx.formula-dependencies"`, version 1.

- [ ] **Step 4:** `cargo test -p dotall-xlsx --test dependencies`

- [ ] **Step 5: Commit** `feat(xlsx): build formula dependency graphs`

---

### Task 5: Wire into format + Engine ensure path

**Files:** Modify `format.rs`, `orchestrate/mod.rs`, optionally `projection.rs`.

- [ ] **Step 1: Tests** — second identical deps query is derived-cache hit (parse/build count stays 1); model/source change rebuilds.

- [ ] **Step 2:** Add `XlsxFormat` method or module fn `ensure_formula_dependencies(store, relative, model_envelope) -> DependencyGraph` that reads cache by recipe else builds, writes `CachedDerived`, returns graph.

- [ ] **Step 3:** Engine API e.g. `dependencies(&mut self, relative, query) -> Result<DepsResult>` **or** extend `read` with selector kinds `deps_forward` / `deps_reverse`. Prefer dedicated Engine method + CLI for clarity; still format-agnostic via a small `FormatHandler` extension **or** xlsx-only path in CLI that calls xlsx helper after Engine loads model.

  **Decision for this plan:** Keep `FormatHandler` unchanged; CLI/Engine loads model via existing inspect/model path, then calls `dotall_xlsx::dependencies::ensure_and_query(...)`. Avoid core→xlsx. Engine may expose `model_for` internals or CLI duplicates freshness+model load via public Engine methods — prefer adding `Engine::load_model(&mut self, relative) -> Result<(ArtifactEnvelope, source_hash, cache_hit)>` if not already accessible.

- [ ] **Step 4: Commit** `feat(xlsx): cache and query formula dependencies`

---

### Task 6: CLI dependency commands

**Files:** Modify `dotall-cli`; create `tests/xlsx_dependencies.rs`.

- [ ] **Step 1: Tests** — `dotall deps book.xlsx --cell 'Revenue!B4'` (forward) and `--dependents` (reverse); `--json`; warm call does not rewrite derived file mtime.

- [ ] **Step 2: Implement** commands under feature `xlsx`.

- [ ] **Step 3:** `cargo test -p dotall-cli`

- [ ] **Step 4: Commit** `feat(cli): query formula dependencies`

---

### Task 7: Quality gate

```bash
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo check -p dotall-cli --no-default-features
```

- [ ] Fix failures; commit only if needed: `chore: formula-deps quality gate`

---

## Self-review

- No evaluation engine.
- Uses existing derived cache directory.
- Write/history design Phase 0 satisfied (graph + queries + invalidation hooks).
- Core stays free of `dotall-xlsx` dependency.
