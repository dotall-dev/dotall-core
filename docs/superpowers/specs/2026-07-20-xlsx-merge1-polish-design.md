# XLSX Merge 1 Polish Design

**Date:** 2026-07-20  
**Status:** Approved  
**PR:** https://github.com/dotall-dev/dotall-core/pull/4  
**Related:** `docs/superpowers/specs/2026-07-20-xlsx-write-history-design.md`

## Goal

Close Merge 1 fidelity and audit gaps on the existing transactional edit spine before Merge 2 structural ops. All work lands on PR #4.

## Scope

| Item | Decision |
|------|----------|
| Discard audit | Write cancel audit under `state/transactions/`; no history version |
| ZIP goldens | Assert CRC, compression method, compressed size; raw compressed bytes when API allows |
| String cells | Prefer `sharedStrings.xml` mutation when present; `inlineStr` only if no sst part |
| Tests/docs | Engine discard + incomplete-journal coverage; roadmap notes Merge 1 polish complete |

## Non-goals

- Merge 2 structural ops  
- MCP session-close flush  
- Formula evaluation  
- Shared-formula / array-formula editing (remain reject)

## 1. Cancel audit

On `Engine::discard` / `discard_staged`:

1. If staged tx exists, write `state/transactions/<tx_id>.cancel.json` with `{ tx_id, status: "cancelled", timestamp, actor, reason: "discard" }`.
2. Remove `state/edits/staging/<tx_id>.json`.
3. Do **not** call `append_history`.

Idempotent discard of missing tx: no-op or clear error (prefer clear `HistoryVersionMissing`-style / staged-not-found).

## 2. Surgical ZIP fidelity asserts

Golden helper must, for every untouched entry:

- CRC32 equal  
- compression method equal  
- compressed size equal  
- inflated bytes equal  
- when available: raw compressed local payload equal  

Every surgical test (value, formula, sparse, string, escaped sheet name) must call this helper with the correct patched-parts allowlist. Styles / other worksheets must remain in the untouched set when not edited.

## 3. Shared strings

When applying `set_cell_value` with a string:

1. If package contains `xl/sharedStrings.xml` (or the workbook relationship target for shared strings):
   - Parse existing `<si>` entries.
   - Reuse index if exact string already present; else append.
   - Update `count` / `uniqueCount` attributes on `<sst>`.
   - Emit cell as `t="s"` with `<v>{index}</v>`.
   - Include shared strings part in ZIP replacements (may lose byte-identity for that part only).
2. If no shared strings part exists: keep `inlineStr` path (no sst creation required for Merge 1 polish unless a fixture needs it).
3. Replacing an existing `t="s"` cell: point at new/reused index; do not rewrite unrelated `<si>` entries.

Untouched ZIP entries (including other worksheets, styles, charts) remain byte-identical except the intentionally patched worksheet(s) and sst when mutated.

## 4. Docs

Update v0 roadmap / write-history notes: Merge 1 cell spine + polish complete; next is Merge 2 structural ops plan.
