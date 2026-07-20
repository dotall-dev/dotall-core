# XLSX Merge 2 Structural Edits Design

**Date:** 2026-07-20  
**Status:** Approved  
**Delivery:** Full Merge 2 (impact → set_range → rows/cols → sheets)  
**Related:**  
- `docs/superpowers/specs/2026-07-20-xlsx-write-history-design.md` §8  
- `docs/superpowers/plans/2026-07-18-broaden-xlsx-edit-operations.md` (superseded by revised plan)

## Goal

Extend the Merge 1 stage/apply/history spine with structural XLSX operations while never silently leaving stale OOXML references.

## Non-negotiable rule

> Patch every affected reference Dotall understands, or reject before source mutation with the exact unsupported construct and part path.

Charts, pivots, and VBA remain preserve-only (reject when an op would require mutating them).

## Spine (unchanged)

Core `Engine::edit` / `apply` / `discard` / `history` / `diff` / `revert` unchanged. Format handler `validate_edit` / `apply_edit` grow new op kinds. Capabilities advertise an op only after its golden tests pass.

## Delivery order

1. Impact inventory + reject unsupported  
2. `set_range`  
3. Coordinate transforms (pure)  
4. Insert/delete rows  
5. Insert/delete columns  
6. Add/rename sheets  
7. Delete sheets (policies)  
8. CLI + quality gate  

## Invariants

- Stage-only until apply  
- Untouched ZIP entries byte-identical except intentionally patched parts  
- Formula strings rewritten by reference spans only — no evaluation  
- Revert restores snapshot bytes  

## Merge 1 API anchors

- Writer: `dotall-xlsx/src/edits/writer/{package,worksheet,shared_strings}.rs`  
- Validate: `edits/validate.rs`, ops in `edits/ops.rs`  
- Deps: `dependencies/graph.rs` (`build`, `reverse_at`)  
- Surgical goldens: `tests/surgical_edit.rs` fidelity helper  
