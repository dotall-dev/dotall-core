# Killer Demo Design — Dotall vs no-Dotall

**Date:** 2026-08-17  
**Status:** Approved for planning  
**Related:**  
- `docs/specs/dotall-overview.md`  
- `docs/specs/xlsx-engine-v0.md` (`.all/` layout)  
- `docs/superpowers/specs/2026-07-21-mcp-agent-interface-design.md`  
- `docs/superpowers/specs/2026-08-12-snapshot-delta-storage-design.md`  
- `docs/superpowers/plans/2026-08-17-format-parity-demo.md`

## Goal

Ship an **investor-facing bake-off** (~60–90s) that is fully honest: a live Cursor
agent with Dotall MCP vs a live agent without it, on the **same Q3 board-pack
brief**, plus a coda HTML visualizer of the real `.all/` directory (tree + metrics
we can actually compute). Dotall wins by **orchestrating across four files**
(xlsx → pptx → docx → pdf) using cached agent views, not by faking chat, speeding
up the naive take, or baking a static snapshot.

## Decisions

| Topic | Choice |
|-------|--------|
| Audience | Investors / homepage wow (~60–90s) |
| Beats | All three: surgical fidelity, multi-file orchestration, `.all/` time travel |
| Honesty | Live MCP; no fake overlay; no sped-up naive footage; no baked viz HTML |
| Naive toolkit | Python + unzip allowed (what real agents do) |
| Story | Q3 board pack; workbook is source of truth |
| Search | New `dotall search` / `dotall_search` over **cached views/models**, never ZIP XML |
| Visualizer | `dotall viz` loopback HTTP server over **live** `.all/`; tree, not animation |
| Multi-file apply | **Not v1** — four files, one agent session, per-file `apply` |
| Formula engine | **Out of scope** — agent quotes cells it `read`; Excel recalc is a native-app beat |

## 1. The brief (identical both sides)

> Prepare the Q3 board pack. Commission rate is now 15%. Update the workbook, then
> make the deck KPIs, the memo, and the intake form match those figures. Do not
> break formulas, the metrics table, or Word styles.

Success: the four files **agree** on 15% (and on any figures the agent actually
read from the workbook). If the deck still says 10% after Excel says 15%, the
take failed.

This is **orchestration**, not four independent edits: spreadsheet → slides →
memo → routing-slip PDF.

Cached formula results in the `.xlsx` stay stale until Excel opens. That is the
fidelity beat (formulas preserved), not live recalc. The agent must not claim
Commission updated in-file until a human opens Excel or it writes an explicit
value it computed.

## 2. Heavy Q3 pack

Today’s `demo/financials.xlsx`, `demo/deck.pptx`, `demo/memo.docx`, and
`demo/form.pdf` remain the **capability smoke book** (per-wave CLI). They are too
small to drown a naive agent.

Add a generated pack under `demo/q3-pack/`, produced by the existing
`generate_demos` example (or a sibling example invoked from it):

| File | Requirements |
|------|----------------|
| `q3-financials.xlsx` | Many sheets + long history tabs; named range `Rate` → live input cell; commission formula; decoy “10%” in old years / comments / unused labels |
| `q3-deck.pptx` | Many slides; live KPI is **one** shape or table cell; appendix slides contain decoy 10% |
| `q3-memo.docx` | Many paragraphs + a status table; **only** the Q3 status line should change |
| `q3-intake.pdf` | AcroForm fields that must copy rate / owner / date from the pack |

Generator is deterministic. Smoke asserts: `Rate` is a named range; decoy `10%`
exists **outside** the live KPI targets.

## 3. Workspace search (unfair advantage)

New workspace-level search over **already materialized** `.all/` cache:

- CLI: `dotall search <query> [--glob <pattern>] [workspace]`
- MCP: `dotall_search` with `{ "query": string, "glob": string | null }`

**Scan:** `cache/views/**` and `cache/model/**` JSON/Markdown under each tracked
object. **Do not** open source ZIP parts, `state/edits/snapshots/parts/`, or
binary payloads.

**Cold start:** if views/models are missing, return hits only for indexed
objects, plus an explicit `not_indexed` list (paths that need `inspect`/`read`
first). Do not silently unzip the source to “help.”

**Hit shape:**

```json
{
  "path": "q3-pack/q3-financials.xlsx",
  "format_id": "xlsx",
  "selector_kind": "named_ranges",
  "selector": "Rate",
  "snippet": "Rate → Inputs!$B$2"
}
```

`selector_kind` / `selector` must be enough for a follow-up `dotall_read` or
`dotall_edit` without grepping XML. If a hit is inside a view blob with no
stable selector, omit selector fields and still return `path` + `snippet`.

Glob limits relative paths (e.g. `q3-pack/*`).

This is why naive unzip+grep is slow and Dotall is instant after the first
inspect.

## 4. Dotall agent loop

```text
dotall_init
  → status / inspect each pack file
  → search "10%" / Rate
  → read named Rate + quote cells
  → surgical edits on all four files
  → apply (or flush-on-close)
  → history
  → dotall viz
```

Edits use **existing** ops: `set_cell_value` on `Rate`; `replace_shape_text` /
`set_table_cell_text`; `replace_paragraph_text`; `set_form_fields`; plus one
visible run style (bold/color) so native apps look updated.

Skills (`skills/*/SKILL.md`) mention `dotall_search` as the pack needle-finder
and tell agents not to unzip OOXML.

## 5. `dotall viz`

`dotall viz [workspace]` starts a **loopback** HTTP server, opens the default
browser, serves one HTML+JS page. Read-only over `.all/`. Does not mutate
store, source files, or history.

**Reads:** `manifest.json`; per-object `meta.json`; directory tree; 
`state/access/log.jsonl`; `state/edits/history/vNNN.json`; snapshot manifests
and part file sizes (hashes + bytes, not part payloads in the default view).

**UI:** expandable tree (`cache/` vs `state/`); per-file history (`v001…` + op
summaries); click-to-preview JSON/JSONL truncated.

**Honest metrics only:**

| Metric | Source |
|--------|--------|
| Bytes on disk | Walk `.all/` and per-object dirs |
| Snapshot storage saved | Unique part hashes’ bytes vs naive `version_count × source_file_bytes` (OOXML); PDF uses opaque blob hashes the same way |
| Cache hits | Count `model_cache_hit` / `view_cache_hit` in access logs |
| Tokens served vs dump | Sum `estimated_tokens` from access log vs 4-chars-per-token estimate of concatenating extracted text/XML from the four **source** files (computed at viz time; labeled as estimate) |

Do **not** display invented wall-clock “minutes saved.” That comparison is the
filmed sessions.

HTTP JSON endpoints (names locked for tests): `GET /` (HTML), `GET /api/tree`,
`GET /api/metrics`. Bind `127.0.0.1` with an ephemeral port.

## 6. Bake-off filming

1. Naive Cursor session, same brief, **no** Dotall MCP, Python+unzip allowed.  
2. Dotall session on a **copy** of the same pack + `dotall init`.  
3. Open the four Dotall outputs in Excel / PowerPoint / Word / Preview.  
4. `dotall viz` on that workspace.

If the Dotall agent stalls, keep the take. Do not splice CLI into the MCP
session.

Filming notes live in `demo/q3-pack/README.md` (brief, expected hits, viz
command). They are instructions, not a scripted fake agent.

## 7. Testing

- **Search:** indexed views → hit includes usable selector when one exists; missing
  cache → object listed in `not_indexed`, no ZIP scan; glob excludes other files;
  query is substring match, case-insensitive.
- **Viz:** temp `.all/` with two objects and one history record → `/api/tree` lists
  them, `/api/metrics` reports byte counts and cache-hit fields; `GET /` returns
  HTML.
- **Pack generator:** deterministic; `Rate` named; decoy `10%` exists outside live
  KPI targets.

## 8. Error handling

- Search with empty query → CLI/MCP error, no full scan.  
- Viz if `.all/` missing → exit with the same class of error as `status` on an
  uninitialized workspace.  
- Viz never writes. Preview of huge JSON is size-capped (truncate + note).  
- Search never follows symlinks out of `.all/objects/`.

## Non-goals

- Formula evaluation engine  
- Watch daemon / live-updating viz  
- Static HTML export for the marketing site  
- Homepage hosting  
- One `apply` across four files / pack-as-single-object  
- Multi-agent locking  
- Charts / pivots **editing** (preserve only)  
- Invented time-saved minutes in the UI  

## Implementation note

Core search + viz live in `dotall-core` (walk store paths) with thin CLI/MCP
adapters. Pack generation stays in `dotall-cli` examples + format demo helpers.
Surgical edit ops are already on `feat/format-parity-demo`; this workstream does
not depend on new format ops beyond search/viz/pack fixtures.
