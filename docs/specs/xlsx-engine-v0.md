# XLSX Engine v0 — Implementation Spec

First buildable milestone for the Dotall engine. Pure-Rust core, CLI-first, XLSX as
the lead format. Wraps into an MCP server once the core is solid.

See `docs/specs/dotall-overview.md` for product context.

## Guiding principles

- **Agent-readability first.** Humans keep using Excel for the original; the
  artifacts we generate are optimized for LLM consumption.
- **Modular, plug-and-play.** Each concern is an isolated crate behind a narrow
  interface. Formats plug in via a `Format` trait — new formats are new crates,
  never core rewrites.
- **Own the pipeline, integrate the parsers.** Own the IR, cache, graph, views, and
  edit/version pipeline. Integrate existing parsers first; replace format-by-format.
- **YAGNI.** Ship the thin vertical slice, then broaden.

## 1. Definition of done

On a workspace containing `financials.xlsx`, the full loop works — driven by a CLI
harness first, then the same core exposed via MCP:

```
dotall init             # create .all/, register workspace
dotall read <file>      # cold: parse → cache; warm: cache hit (no reparse)
dotall edit <file> ...  # stage a composable edit op
dotall apply <file>     # round-trip into the real .xlsx, record version
dotall history | diff | revert <file>
dotall status           # tracked objects, fresh/stale, versions
```

**Success criteria:**

- [ ] Second `read` is a cache hit — no reparse.
- [ ] An edited formula round-trips into a valid `.xlsx` that Excel opens cleanly.
- [ ] All untouched content preserved byte-for-byte (styles, charts, other sheets).
- [ ] `revert` restores the prior version.
- [ ] Same core wrapped as an MCP binary, registered in Claude Desktop.

## 2. Workspace layout (one Cargo workspace)

```
dotall/
├── Cargo.toml               # [workspace]
├── crates/
│   ├── dotall-core/         # lib: orchestrates the crates below
│   ├── dotall-cache/        # .all/ IO, manifest, hashing, invalidation
│   ├── dotall-model/        # canonical AST types + JSON (de)serialize, element IDs
│   ├── dotall-semantics/    # formula dependency graph
│   ├── dotall-views/        # L0 summary + Markdown projection + range slices
│   ├── dotall-edit/         # op types, staging, apply, version/diff/revert
│   ├── dotall-xlsx/         # format module: read (calamine) + surgical OOXML write
│   ├── dotall-cli/          # bin: `dotall` (dev harness + real CLI)
│   └── dotall-mcp/          # bin: stdio MCP server (added after core is solid)
```

**Format-module trait from day one:**

```rust
trait Format {
    fn parse(&self, bytes: &[u8]) -> Result<Model>;
    fn summary(&self, model: &Model) -> Summary;
    fn apply_ops(&self, source: &Path, ops: &[EditOp]) -> Result<Vec<u8>>;
}
```

DOCX/PDF later implement `Format` in their own crates.

## 3. `.all/` on-disk layout

Project-local `.all/` only for v0. Global `~/.all/cache/` is deferred.

```
.all/
├── manifest.json                     # tracked objects, config, schema version
└── objects/financials.xlsx/
    ├── meta.json                     # mime, size, mtime, source hash, schema ver
    ├── original.ref                  # pointer to source path + hash (no copy)
    ├── ast/model.json                # ① canonical AST (source of truth for edits)
    ├── graph/deps.json               # ② formula dependency graph
    ├── views/
    │   ├── summary.json              # L0 summary
    │   └── sheets/<sheet>.md         # LLM-friendly Markdown projection
    ├── access/log.jsonl              # read provenance (what served, when, tokens)
    └── edits/
        ├── staging/patch-*.json      # proposed ops, pre-apply
        └── history/
            ├── v001.json ...         # {ops, semantic_diff, ts, agent_id}
            └── snapshots/v001.ref    # pre-apply source hash (revert target)
```

## 4. The three artifacts

### ① Canonical AST — `ast/model.json`

`Workbook → Sheet → Row → Cell`. Each cell carries a stable `element_id`, value,
**opaque formula string**, style ref, number format. Round-trip capable. Mutated by
edits. Not shown to the agent directly.

### ② Semantic graph — `graph/deps.json`

Formula **reference** parsing only: `B12 → depends on → A1:A10`, cross-sheet refs,
named ranges. **No evaluation engine in v0.** Powers "what feeds this cell?" queries
and semantic diffs.

### ③ Projections — `views/`

What the agent consumes:

- `summary.json` (L0, ~200 tokens): sheet list, dims, detected header rows, formula
  count, named ranges, key cells.
- `sheets/<sheet>.md`: clean Markdown table per sheet with formula annotations and
  dependency hints — directly openable by the agent, or served by the `read` tool.

## 5. Read semantics — progressive disclosure + budget

`read(file, scope = summary | sheet | range | full, range?, max_tokens?)`

| Scope | Returns | ~Tokens |
|-------|---------|---------|
| `summary` (default) | L0 summary **+ light preview** (first N rows/sheet, capped) | ~500–1.5k |
| `sheet` | full Markdown projection of one sheet | varies |
| `range` | AST/Markdown slice of e.g. `A1:D20` | 1–5k |
| `full` | whole workbook projection | varies |

- Default sits between a bare summary and a full preview: summary plus enough real
  data to be useful.
- `max_tokens` caps any response (truncate + tell the agent how to drill down).
- Cache hit serves projections straight from disk — zero reparse.
- `access/log.jsonl` records what was already sent to avoid re-sending context.

## 6. Edit → apply → version flow

**Composable primitives (v0):** `set_cell_value`, `set_cell_formula`, `set_range`,
`insert_row`, `delete_row`, `insert_col`, `delete_col`, `add_sheet`, `rename_sheet`,
`delete_sheet`. Broaden to formatting/merges/etc. after the loop is proven.

```
edit  → validate against AST + deps → write edits/staging/patch-NNN.json
apply → snapshot source hash → surgical OOXML write → refresh ast/graph/views
      → append edits/history/vNNN.json (ops + semantic diff)
```

- Auto-apply default (agents); `--stage-only` keeps patches for human review.
- `revert vNNN` restores via snapshot ref.
- Diffs are **semantic when possible** ("B12 formula changed; deps unchanged"), not
  XML byte diffs.

## 7. Round-trip fidelity — the crux

Writes use **surgical OOXML patching**, not full re-serialization:

- Read the model with `calamine`.
- On apply, edit the original `.xlsx` (a ZIP of XML) **at the part level** — patch
  only target cells in `sheetN.xml`, update `sharedStrings.xml` / calc chain as
  needed, and leave every untouched part byte-for-byte (styles, charts, other sheets,
  pivots).

This preserves everything we didn't edit — the "agents break spreadsheets, we don't"
promise. Full-reserialize libraries (e.g. `umya-spreadsheet`) are simpler but risk
silently dropping features we don't yet model. Surgical patching is the edit path; a
fidelity test suite guards it.

## 8. Hashing / invalidation

- `blake3` for content hashing; store `(mtime, size, hash)` in `meta.json`.
- Fast path: mtime + size unchanged → cache fresh, skip hashing.
- Changed → rehash; on mismatch invalidate `ast/graph/views`, **keep**
  `edits/history` (audit trail).
- `dotall status` / `sync` for manual rescan. `watch` daemon deferred.

> Supersedes an earlier "SHA-256 per call" idea: blake3 + mtime fast-path gives the
> same guarantee without rehashing large files on every tool call.

## 9. Testing

- **Round-trip golden tests**: corpus of real `.xlsx` (formulas, styles, charts,
  multi-sheet, named ranges) → open, apply a no-op and a targeted edit, assert
  untouched parts are byte-identical and the edit is correct.
- Formula-ref graph unit tests.
- Cache hit/miss + invalidation tests.
- Snapshot / revert tests.

## 10. Build sequence

1. Workspace + `.all/` core (manifest, meta, hashing) + `dotall init` / `status`.
2. `dotall-xlsx` read (calamine) → `ast/model.json`.
3. `dotall-views`: L0 summary + Markdown projection → `read` (cache hit/miss).
4. `dotall-semantics`: formula dependency graph.
5. `dotall-edit`: staging + composable ops + **surgical OOXML apply** + versioning.
6. `history` / `diff` / `revert`.
7. Broaden edit ops (formatting, merges, etc.).
8. `dotall-mcp`: wrap core as stdio MCP; register in Claude Desktop.

## 11. Non-goals for v0 (YAGNI)

No formula evaluation engine · no global `~/.all/cache` · no `watch` daemon · no
multi-agent conflict/locking · no branching history · no DOCX/PDF · no Python/JS SDK
· no charts/pivots **editing** (preserve them, don't mutate).

## Dependencies (initial, integrate-first)

| Need | Crate (candidate) |
|------|-------------------|
| XLSX read | `calamine` |
| ZIP/OOXML part access | `zip` + `quick-xml` |
| Hashing | `blake3` |
| Serialization | `serde` + `serde_json` |
| CLI | `clap` |
| MCP (later) | Rust MCP SDK (stdio) |

Exact versions pinned at implementation time via `cargo add`.
