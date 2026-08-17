# Killer demo final fix report

**Branch:** `feat/format-parity-demo`  
**Date:** 2026-08-17  
**Scope:** Merge-blocker fixes C1–C2 and I1–I7 after Tasks 1–8.

## Fixes

| ID | Change |
|----|--------|
| **C1** | `search_model` recursively walks model JSON string values (not only `named_ranges`). Named-range entries still get `selector_kind=named_ranges`. Inspect-only packs now hit `"10%"` for xlsx/pptx/docx/pdf. |
| **C2** | CLI test clippy: `KillChild` Drop guard (kill+wait) in `tests/viz.rs`; `is_none_or` in `tests/search.rs`. |
| **I1** | Pin `DocProperties` creation datetime to `2026-08-17` in `q3_pack` and `generate_demos`; two-run byte equality test; regenerated `demo/financials.xlsx` + `demo/q3-pack/q3-financials.xlsx`. |
| **I2** | `matching_lines` returns all matching view lines; cap **32 hits per file**. |
| **I3** | `estimated_dump_tokens` docs + HTML: compressed-bytes/4 **lower bound**. |
| **I4** | HTML metrics: human labels + units; one-line estimate note. |
| **I5** | Core viz test: two objects + history + cache-hit fields; CLI viz asserts `/api/tree` has `"tree"` and `"history"`. |
| **I6** | `ObjectState::Stale` / `Missing` → `not_indexed` (no stale hits). |
| **I7** | Corrupt view JSON / bad access-log lines skipped; search/viz continue. |

## Covering tests

```bash
cargo test -p dotall-core --test search --test viz
cargo test -p dotall-core --lib search
cargo test -p dotall-cli --test search --test viz --test q3_pack
cargo fmt
cargo clippy -p dotall-cli --all-targets -- -D warnings
cargo clippy -p dotall-core -- -D warnings
```

### Results (this wave)

**`cargo test -p dotall-core --test search --test viz`**

```
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

**`cargo test -p dotall-cli --test search --test viz --test q3_pack`**

```
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out  # q3_pack
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out  # search
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out  # viz
```

**`cargo clippy -p dotall-cli --all-targets -- -D warnings`** — Finished (clean)  
**`cargo clippy -p dotall-core -- -D warnings`** — Finished (clean)

### Manual smoke (inspect → search `10%`)

After `dotall init` + `inspect` on all four `demo/q3-pack` files:

- `hits` nonempty for formats: `docx`, `pdf`, `pptx`, `xlsx`
- `not_indexed`: `[]`

## Remaining concerns

- Model string walk can emit duplicate snippets when the same string appears many times in a payload (e.g. repeated decoy cells / shape text); per-file hit cap mitigates spam but does not dedupe.
- Dump-token estimate remains compressed-size/4 by design (no ZIP dep in core); UI now labels it as a lower bound.
- PPTX/DOCX/PDF demo bytes are not datetime-pinned the same way as xlsx (only rust_xlsxwriter creation datetime was in scope).

## Out of scope (unchanged)

No `zip` crate in core · no formula eval · no merge.
