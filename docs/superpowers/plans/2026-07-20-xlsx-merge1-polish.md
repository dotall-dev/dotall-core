# XLSX Merge 1 Polish Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Close Merge 1 audit + surgical fidelity gaps (cancel journal, stronger ZIP goldens, shared-strings writes) on PR #4.

**Spec authority:** `docs/superpowers/specs/2026-07-20-xlsx-merge1-polish-design.md`

**Worktree:** `.worktrees/feat-xlsx-transactional-edits` on `feat/xlsx-transactional-edits`

**Commit policy:** Per-task commits authorized; push to origin updates PR #4.

---

### Task 1: Cancel audit on discard

**Files:** `store/mod.rs`, `history/` or journal helpers, `tests/edit_persistence.rs` / `transactions.rs`

- [ ] **Step 1: Tests** — discard writes `state/transactions/<tx_id>.cancel.json`; history list unchanged; discard missing tx errors cleanly.
- [ ] **Step 2: Implement** cancel audit write before removing staging file; call from `discard_staged` or `Engine::discard`.
- [ ] **Step 3: Commit** `feat(core): audit cancelled staged edits`

---

### Task 2: Strengthen surgical ZIP fidelity asserts

**Files:** `crates/dotall-xlsx/tests/surgical_edit.rs`

- [ ] **Step 1: Extend helper** — assert CRC, compression method, compressed size, inflated bytes; raw compressed bytes if zip API allows.
- [ ] **Step 2: Apply helper** to every surgical test with correct patched-parts allowlist (incl. string/sparse paths).
- [ ] **Step 3: Commit** `test(xlsx): strengthen surgical ZIP fidelity asserts`

---

### Task 3: Shared-strings surgical writes

**Files:** `edits/writer/{package,worksheet}.rs` (+ maybe `shared_strings.rs`); `tests/surgical_edit.rs`

- [ ] **Step 1: Tests** — workbook with existing sst; set string cell → cell uses `t="s"`; sst gains/reuses entry; other ZIP parts byte-identical; no-sst workbook still uses inlineStr.
- [ ] **Step 2: Implement** parse/append sst; wire package replacements; prefer reuse of identical string.
- [ ] **Step 3: Commit** `feat(xlsx): patch shared strings for string cell edits`

---

### Task 4: Docs + Engine coverage gaps

**Files:** roadmap / write-history notes; `tests/transactions.rs` if discard Engine path thin

- [x] **Step 1: Tests** — Engine discard leaves cancel audit and empty staged list (`transactions.rs::discard_writes_cancel_audit_and_clears_staged`).
- [x] **Step 2: Docs** — mark Merge 1 + polish done; next = Merge 2 broaden plan.
- [x] **Step 3: Commit** `docs: mark Merge 1 polish complete`

---

### Task 5: Quality gate + push PR #4

```bash
CARGO_TARGET_DIR=target cargo fmt --all --check
CARGO_TARGET_DIR=target cargo test --workspace
CARGO_TARGET_DIR=target cargo clippy --workspace --all-targets --all-features -- -D warnings
git push origin feat/xlsx-transactional-edits
```

- [ ] Commit fixes if needed: `chore: Merge 1 polish quality gate`
