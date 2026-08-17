# Audit Correctness Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the audit's correctness, agent-DX, and docs holes with TDD so shipped consumers match the spec without breaking existing tests.

**Architecture:** Keep the Engine/store/handler split. Wire existing `recover()` into apply and model. Teach search to emit the same selector kinds formats already understand. Detection reads package evidence when the source path is a real file. Docs and CI catch up to four formats.

**Tech Stack:** Rust 2024, cargo test, existing `tempfile` fixtures, GitHub Actions.

## Global Constraints

- TDD: no production code without a failing test first (docs/CI/config excepted).
- Do not commit unless the user asks.
- No unwrap in library `src/`.
- Do not change `.all/` object-key encoding (breaking).
- Do not extract ZIP rebuild or split Engine in this pass.
- Search selectors must be valid follow-up `read` kinds already advertised by each format.

---

### Task 1: Auto-recover interrupted applies

**Files:**
- Modify: `crates/dotall-core/tests/transactions.rs`
- Modify: `crates/dotall-core/src/orchestrate/mod.rs`

- [ ] Failing tests: apply and inspect after `simulate_interruption_after_replace` without calling `recover()`.
- [ ] Implement: `apply_inner` and `model` call `recover()` first; `recover` loads the handler via `detect` instead of `model` to avoid recursion.

### Task 2: Preserve disk `version_count` on `register_source`

**Files:**
- Modify: `crates/dotall-core/tests/edit_persistence.rs`
- Modify: `crates/dotall-core/src/store/mod.rs`

- [ ] Failing test: store A appends history (version_count=1); store B opened earlier with 0 calls `register_source`; disk must keep 1.
- [ ] Implement: reload manifest from disk and take `max(memory, disk)` version_count.

### Task 3: Do not unlink the apply lock file

**Files:**
- Modify: `crates/dotall-core/tests/edit_persistence.rs`
- Modify: `crates/dotall-core/src/history/lock.rs`

- [ ] Failing test: after drop, `.apply.lock` still exists; a later acquire still succeeds.
- [ ] Implement: unlock in Drop, do not `remove_file`.

### Task 4: Search selectors for cells, slides, paragraphs, fields

**Files:**
- Modify: `crates/dotall-core/tests/search.rs`
- Modify: `crates/dotall-core/src/search.rs`

- [ ] Failing tests for xlsx `range`, pptx `slide`, docx `paragraphs`, pdf `field`.
- [ ] Implement path-aware hits; keep named_ranges; stringify numbers so `0.1` matches.

### Task 5: Package-evidence detection

**Files:**
- Modify: `crates/dotall-xlsx/src/detection.rs` (+ pptx/docx)
- Modify: `crates/dotall-core/src/orchestrate/mod.rs` (pass real source path)

- [ ] Failing test: `.xlsx` whose ZIP contains `ppt/presentation.xml` scores below PPTX.
- [ ] Implement: when the path is a readable ZIP, `zip_contains_entry` boosts the matching family.

### Task 6: Engine XLSX token budget + invalid continuation

**Files:**
- Modify: `crates/dotall-xlsx/tests/read_contract.rs` or core budget tests
- Modify: `crates/dotall-core/src/read/budget.rs`

- [ ] Prove Engine truncates XLSX `full` reads.
- [ ] Invalid UTF-8 byte offset returns an error, not an empty success.

### Task 7: Status panic, FreshAfterHash write-back, PDF signatures, MCP hash, CLI --op, generate_demos, docs, CI

Docs/CI have no failing tests. Behavioral items still get tests first.

---
