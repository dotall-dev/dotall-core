# Transactional XLSX Edits (Merge 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stage-only cell value/formula edits with surgical OOXML apply, optimistic concurrency, idempotent transactions, forensic history, semantic diff, crash recovery, and revert — Merge 1 of the write/history design.

**Architecture:** Core owns stage/apply journals, locks, snapshots, history records, and Engine edit orchestration. `dotall-xlsx` owns typed ops, validation (including dependency impact), surgical ZIP/XML patching, and semantic diffs. Default is **stage**; source mutates only on `apply` (CLI) or later MCP session-close flush.

**Tech Stack:** Existing core + formula-deps, `uuid`, `zip`, `quick-xml`, `tempfile`, `calamine` for post-apply checks.

**Spec authority:** `docs/superpowers/specs/2026-07-20-xlsx-write-history-design.md`. Supersedes auto-apply defaults in `2026-07-18-transactional-xlsx-edits.md`.

**Prerequisite:** `2026-07-20-xlsx-formula-dependencies.md` complete.

**Commit policy:** Per-task commits authorized when executing this plan.

---

## Invariants

- No source mutation before validate + durable stage intent; apply requires snapshot + journal.
- `expected_source_hash` checked on edit and re-checked on apply.
- One local writer lock per object during apply.
- Idempotent `tx_id`.
- History append-only; revert creates a **new** version.
- Failed validate/apply leaves source unchanged.
- Untouched OOXML ZIP entries remain byte-identical after surgical patch.

## Target files

```text
crates/dotall-core/src/history/{mod.rs,record.rs,transaction.rs,lock.rs}
crates/dotall-core/src/registry/{mod.rs,types.rs}   # validate_edit / apply hooks
crates/dotall-core/src/orchestrate/mod.rs
crates/dotall-core/src/store/mod.rs
crates/dotall-core/tests/transactions.rs
crates/dotall-xlsx/src/edits/{mod.rs,ops.rs,validate.rs,writer/{mod,package,worksheet}.rs}
crates/dotall-xlsx/tests/surgical_edit.rs
crates/dotall-cli/src/main.rs
crates/dotall-cli/tests/xlsx_edit_history.rs
```

---

### Task 1: Edit / history contracts

**Files:** `history/*`, `registry/types.rs`, `error.rs`, `lib.rs`; add `uuid` dep.

- [ ] **Step 1: Types**

```rust
EditRequest {
  transaction_id: Uuid,
  expected_source_hash: String,
  actor: Actor,
  operations: Vec<SemanticOperation>, // kind + payload
  // NO auto-apply flag — stage is default; apply is a separate call
}
StagedEdit { tx_id, preview: ValidatedEdit, staged_at, ... }
ValidatedEdit { format_id, schema_id, schema_version, operations, semantic_diff, dependency_impact }
HistoryRecord { /* forensic fields from write design §3 */ }
```

- [ ] **Step 2: Extend `FormatHandler`**

```rust
fn validate_edit(&self, model: &ArtifactEnvelope, ops: &[SemanticOperation]) -> Result<ValidatedEdit>;
fn apply_edit(&self, source: &Path, edit: &ValidatedEdit) -> Result<PatchedOutput>;
```

Module comment: stage/apply owned by Engine; handler only validate + patch.

- [ ] **Step 3: Unit tests** for serde round-trip of HistoryRecord / EditRequest.

- [ ] **Step 4: Commit** `feat(core): define staged edit and history contracts`

---

### Task 2: Staging, journal, lock, snapshot persistence

**Files:** `history/transaction.rs`, `lock.rs`, store helpers.

- [ ] **Step 1: Tests** — stage writes `state/edits/staging/<tx>.json`; duplicate tx_id returns same staged payload; lock prevents concurrent apply; snapshot stored under `history/snapshots/<hash>.bin`.

- [ ] **Step 2: Implement** durable stage/read/discard; advisory file lock; content-addressed snapshot copy.

- [ ] **Step 3: Commit** `feat(core): persist staged edits, locks, and snapshots`

---

### Task 3: History append + agent projections

**Files:** `history/record.rs`, store history APIs.

- [ ] **Step 1: Tests** — append v1/v2; list compact history; load forensic record; cancel does not create history version.

- [ ] **Step 2: Implement** `append_history`, `list_history`, `get_history`; compact vs full views.

- [ ] **Step 3: Commit** `feat(core): append forensic edit history`

---

### Task 4: XLSX cell ops + validate with deps

**Files:** `dotall-xlsx/src/edits/{ops,validate}.rs`.

- [ ] **Step 1: Tests** — validate `set_cell_value` / `set_cell_formula`; reject unknown sheet; include dependency_impact from formula graph; semantic_diff before/after.

- [ ] **Step 2: Implement** typed ops + validate against `WorkbookModel` + deps graph.

- [ ] **Step 3: Commit** `feat(xlsx): validate cell value and formula edits`

---

### Task 5: Surgical OOXML writer

**Files:** `edits/writer/*`; `tests/surgical_edit.rs`.

- [ ] **Step 1: Golden tests** — edit one cell; assert target value/formula correct; assert untouched ZIP entries byte-identical (compare entry CRCs/bytes); charts/styles parts unchanged if present.

- [ ] **Step 2: Implement** ZIP copy + patch worksheet (+ shared strings as needed); atomic temp replace.

- [ ] **Step 3: Commit** `feat(xlsx): surgically patch cell values and formulas`

---

### Task 6: Engine stage / apply / revert

**Files:** `orchestrate/mod.rs`; `tests/transactions.rs`.

- [ ] **Step 1: Tests**

  - `edit` stages; source hash unchanged
  - `apply` mutates source, appends history, refreshes model
  - stale `expected_source_hash` rejects apply
  - idempotent tx_id
  - `revert` stages restore; apply creates new version with `revert_of`
  - crash: kill after journal “commit” mark recovers cleanly (or leaves source unchanged if incomplete)

- [ ] **Step 2: Implement** `Engine::edit`, `apply`, `discard`, `history`, `diff`, `revert` following write design lifecycle. After apply: re-fingerprint, update manifest, invalidate views/derived, rewrite model.

- [ ] **Step 3: Commit** `feat(core): orchestrate staged edit apply and revert`

---

### Task 7: CLI

**Files:** `dotall-cli`; `tests/xlsx_edit_history.rs`.

- [ ] **Step 1: Tests** — stage edit; apply; history; diff; revert+apply; `--json`.

- [ ] **Step 2: Commands** — `edit`, `apply`, `discard`, `staged`, `history`, `diff`, `revert`.

- [ ] **Step 3: Commit** `feat(cli): stage and apply XLSX cell edits`

---

### Task 8: Quality gate

```bash
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Plus manual golden: surgical untouched-part check on a multi-sheet fixture with a chart if available.

- [ ] Commit fixes if needed: `chore: transactional edits quality gate`

---

## Merge 2 note

Do not implement structural ops here. After Merge 1, execute/revise `broaden-xlsx-edit-operations` against the same stage/apply/history spine.

## MCP note

Session-close flush is a later MCP plan hook calling `Engine::apply` for remaining staged txs — not implemented in this plan beyond a documented Engine API suitable for that call.
