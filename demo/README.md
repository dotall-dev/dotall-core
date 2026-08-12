# Dotall demo workspace

Sample workbooks for CLI / MCP walkthroughs and the YC demo video.

| File | Role |
|------|------|
| [`saas_model.xlsx`](saas_model.xlsx) | **YC / video** — large multi-sheet SaaS model |
| [`financials.xlsx`](financials.xlsx) | Tiny fixture for quick CLI smoke tests |

## Workbook: `saas_model.xlsx` (YC demo)

Acme SaaS FY2026 operating model.

| Sheet | Size | Contents |
|-------|------|----------|
| `Assumptions` | 11 drivers | ListPrice, MoMGrowth, Churn, COGS%, OpEx, Tax, … |
| `Regions` | 8 regions × 12 months | Jan seed units; Feb–Dec `=prev*(1+Assumptions!$B$3)` |
| `Revenue` | same grid | `=Regions!…*Assumptions!$B$2` |
| `Income` | monthly P&L | Cross-sheet Revenue sums, COGS/OpEx/tax from Assumptions |
| `Board` | KPIs + chart | ARR / MRR / FY Revenue / margins (chart is **preserve-only**) |

**Cross-sheet spine:** `Assumptions!B2/B3/B5` → Regions growth + Revenue price + Income COGS → Board ARR.

### Baseline values (camera)

| Cell | Label | Value |
|------|-------|-------|
| `Assumptions!B2` | ListPrice | `49` |
| `Assumptions!B3` | MoMGrowth | `0.08` |
| `Assumptions!B4` | Churn | `0.02` |
| `Assumptions!B5` | COGS% | `0.22` |
| `Regions!B2:B9` | Jan units | `1200, 960, 800, 400, 320, 560, 640, 280` |

### Big edit (one staged transaction → one apply → one revert)

| Op | Range | After |
|----|-------|-------|
| `set_range` | `Assumptions!B2:B5` | `59`, `0.12`, `0.015`, `0.18` |
| `set_range` | `Regions!B2:B9` | `1500, 1200, 1000, 500, 400, 700, 800, 350` (×1.25) |

### Regenerate

```bash
cargo run -p dotall-xlsx --example gen_yc_demo_workbook
```

## Setup

```bash
cargo build --release -p dotall-cli -p dotall-mcp

./target/release/dotall init demo
./target/release/dotall inspect demo/saas_model.xlsx
```

## CLI walkthrough (YC story)

```bash
DOTALL=./target/release/dotall
cd demo   # or paths relative to repo root

$DOTALL inspect saas_model.xlsx
$DOTALL read saas_model.xlsx --range 'Assumptions!A1:B6'
$DOTALL read saas_model.xlsx --range 'Regions!A1:B9'
$DOTALL read saas_model.xlsx --range 'Board!A1:B7'
$DOTALL deps saas_model.xlsx --cell 'Board!B2'

# One transaction: Assumptions block + Regions Jan bump
$DOTALL edit saas_model.xlsx --ops-json '[
  {"kind":"set_range","payload":{"sheet":"Assumptions","start_cell":"B2","values":[[59],[0.12],[0.015],[0.18]]}},
  {"kind":"set_range","payload":{"sheet":"Regions","start_cell":"B2","values":[[1500],[1200],[1000],[500],[400],[700],[800],[350]]}}
]'

$DOTALL staged saas_model.xlsx
$DOTALL apply saas_model.xlsx --all

# Open saas_model.xlsx in Excel/LibreOffice — changed file on disk
$DOTALL read saas_model.xlsx --range 'Assumptions!A1:B5'
$DOTALL history saas_model.xlsx
$DOTALL diff saas_model.xlsx --version 1

# Surgical restore of the whole transaction
$DOTALL revert saas_model.xlsx --version 1
$DOTALL apply saas_model.xlsx --all

# Reopen Excel — reverted file (baseline again)
$DOTALL read saas_model.xlsx --range 'Assumptions!A1:B5'
$DOTALL read saas_model.xlsx --range 'Regions!B2:B9'
```

**File beats for video:** baseline → after apply (changed) → after revert+apply (restored). Close/reopen Excel after each apply.

Formula results show as `0` in Dotall reads until Excel recalculates — use Excel for KPI proof.

## Tiny walkthrough: `financials.xlsx`

```bash
$DOTALL inspect financials.xlsx
$DOTALL read financials.xlsx --range 'Revenue!A1:B5'
$DOTALL edit financials.xlsx \
  --op set_cell_value --sheet Inputs --address B2 --value 0.15
$DOTALL apply financials.xlsx --all
$DOTALL revert financials.xlsx --version 1
$DOTALL apply financials.xlsx --all
```

## MCP walkthrough (YC)

1. Build: `cargo build --release -p dotall-mcp`
2. Copy [`mcp.example.json`](mcp.example.json) into your MCP client; replace `REPO_ROOT`; `cwd` → `…/demo`.
3. Attach [`skills/xlsx/SKILL.md`](../skills/xlsx/SKILL.md).
4. Paste:

   > Read `skills/xlsx/SKILL.md`. Inspect `saas_model.xlsx`. Briefly show that Assumptions feed Revenue and Board. Stage **one** edit transaction that (1) sets `Assumptions!B2:B5` to 59, 0.12, 0.015, 0.18 and (2) sets `Regions!B2:B9` to 1500, 1200, 1000, 500, 400, 700, 800, 350. **Apply** it. Show history/`diff`. **Revert** that version and **apply** again so the file is restored.

5. Optional: stage without apply and end the session — default flush-on-close commits staged txs (`--no-flush-on-close` / `DOTALL_MCP_FLUSH_ON_CLOSE=0` to contrast).

## Tips

- Prefer the **release** binary for live demos (`cargo run` is slow on camera).
- `.all/` under `demo/` is gitignored — safe to delete and regenerate.
- After apply, close or refresh Excel so you see the real on-disk file.
- Filming: see [`YC_VIDEO.md`](YC_VIDEO.md), rehearse with [`yc_rehearse.sh`](yc_rehearse.sh), VO/export checklist in [`yc-export/VO_AND_EXPORT.md`](yc-export/VO_AND_EXPORT.md).
