# XLSX Engine v0 — Implementation Spec

First buildable milestone for the Dotall engine. Pure-Rust core, CLI-first, XLSX as
the lead format. The same core is exposed via stdio MCP — see
`docs/superpowers/specs/2026-07-21-mcp-agent-interface-design.md` and
`docs/superpowers/plans/2026-07-21-mcp-agent-interface.md`.

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
- [x] Same core wrapped as an MCP binary (`dotall-mcp`); see MCP spec above and
      `skills/xlsx/SKILL.md` for agent workflow.

## 2. Workspace layout (one Cargo workspace)

```
dotall/
├── Cargo.toml               # [workspace]
├── crates/
│   ├── dotall-core/         # store, registry, pipeline, read, history, orchestration
│   ├── dotall-xlsx/         # typed model, processors, views, edits, OOXML writer
│   ├── dotall-cli/          # bin: `dotall` (dev harness + real CLI)
│   └── dotall-mcp/          # bin: stdio MCP server (`dotall-mcp`)
```

Core concerns begin as strict internal modules. Promote one to a separate crate only
when independent dependencies, feature gating, test isolation, or ownership make
the boundary valuable. Do not generalize a universal model or graph from XLSX
alone.

**Format handler contract from day one:**

```rust
trait FormatHandler {
    fn detect(&self, probe: &DetectionProbe) -> DetectionScore;
    fn capabilities(&self) -> Capabilities;
    fn parse(&self, source: &Path) -> Result<ArtifactEnvelope>;
    fn read(&self, model: &ArtifactEnvelope, request: &ReadRequest)
        -> Result<ReadResponse>;
    fn validate_edit(&self, model: &ArtifactEnvelope, ops: &[SemanticOperation])
        -> Result<ValidatedEdit>;
    fn apply_edit(&self, source: &Path, edit: &ValidatedEdit)
        -> Result<PatchedOutput>;
}
```

DOCX/PDF later implement the handler contract in their own format-family crates.

## 3. `.all/` on-disk layout

Project-local `.all/` only for v0. Global `~/.all/cache/` is deferred.

```
.all/
├── manifest.json                     # tracked objects, config, schema version
└── objects/financials.xlsx/
    ├── meta.json                     # mime, size, mtime, source hash, schema ver
    ├── original.ref                  # pointer to source path + hash (no copy)
    ├── cache/                        # safe to delete and regenerate
    │   ├── model/model.json          # ① canonical typed model
    │   ├── derived/                  # ② dependency graph and later processors
    │   └── views/                    # ③ summaries and Markdown projections
    └── state/                        # durable across cache invalidation
        ├── access/log.jsonl          # read provenance
        ├── transactions/             # crash recovery journals
        └── edits/
            ├── staging/              # proposed ops, pre-apply
            └── history/
                ├── v001.json ...     # ops, semantic diff, timestamp, actor
                └── snapshots/        # pre-apply content-addressed snapshots
```

## 4. The three artifacts

### ① Canonical model — `cache/model/model.json`

`Workbook → Sheet → Row → Cell`. Each cell carries a stable `element_id`, value,
**opaque formula string**, style ref, number format. Round-trip capable. Mutated by
edits. Not shown to the agent directly.

### ② Semantic graph — `cache/derived/`

Formula **reference** parsing only: `B12 → depends on → A1:A10`, cross-sheet refs,
named ranges. **No evaluation engine in v0.** Powers "what feeds this cell?" queries
and semantic diffs.

### ③ Projections — `cache/views/`

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
edit  → validate against model + deps → write state/edits/staging/patch-NNN.json
apply → snapshot source hash → surgical OOXML write → refresh model/derived/views
      → append state/edits/history/vNNN.json (ops + semantic diff)
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

1. Core storage foundation: workspace, manifest, hashing, `init`, and `status`.
2. XLSX detection and cached typed-model reads with agent projections.
3. Formula dependency derivation and queries.
4. Transactional value/formula edits, surgical OOXML apply, history, diff, recovery,
   and revert.
5. Broaden row/column/range/sheet operations behind fidelity tests.
6. `dotall-mcp`: expose the same core as a stdio MCP server — **shipped**; see
   `docs/superpowers/specs/2026-07-21-mcp-agent-interface-design.md`.

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
| MCP | `rmcp` (stdio); see MCP agent-interface spec |

Exact versions pinned at implementation time via `cargo add`.
