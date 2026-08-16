# Dotall demo workspace

Multi-format samples for CLI and MCP walkthroughs. Regenerate after format changes:

```bash
cargo run -p dotall-cli --example generate_demos
```

| File | Format | Agent workload |
|------|--------|----------------|
| [`financials.xlsx`](financials.xlsx) | XLSX | Multi-sheet formulas, named range `Rate`, merged header |
| [`deck.pptx`](deck.pptx) | PPTX | Title + metrics table + speaker notes; second slide next-steps |
| [`memo.docx`](memo.docx) | DOCX | Heading memo + status table + confidential header |
| [`form.pdf`](form.pdf) | PDF | Intake form: Name, Email, Agree checkbox, Department choice; `/Info` metadata |

Skills: [`xlsx`](../skills/xlsx/SKILL.md) · [`pptx`](../skills/pptx/SKILL.md) · [`docx`](../skills/docx/SKILL.md) · [`pdf`](../skills/pdf/SKILL.md)

## Setup

```bash
cargo build -p dotall-cli -p dotall-mcp
./target/debug/dotall init demo
```

Use `./target/debug/dotall` below (or a release build). Paths are relative to the repo root unless noted.

---

## XLSX — `financials.xlsx`

| Sheet | Cell | Value / formula |
|-------|------|-----------------|
| `Inputs` | A1:B1 | merged header `Assumptions` |
| `Inputs` | B2 | `0.10` (Rate) — named range `Rate` |
| `Inputs` | B3 | `100` (Base) |
| `Revenue` | B2 / B3 | Jan / Feb amounts |
| `Revenue` | B4 | `=B2+B3` (Total) |
| `Revenue` | B5 | `=B4*Inputs!B2` (Commission) |

```bash
DOTALL=./target/debug/dotall

$DOTALL inspect demo/financials.xlsx
$DOTALL read demo/financials.xlsx --selector-kind named_ranges
$DOTALL read demo/financials.xlsx --selector-kind merges --selector Inputs
$DOTALL read demo/financials.xlsx --range 'Revenue!A1:B5'
$DOTALL deps demo/financials.xlsx --cell 'Revenue!B5'

# Prior v0 path
$DOTALL edit demo/financials.xlsx \
  --op set_cell_value --sheet Inputs --address B2 --value 0.15
$DOTALL apply demo/financials.xlsx --all

# Wave 1 path: set_range (ops-json)
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"set_range","payload":{"sheet":"Revenue","start_cell":"B2","values":[[110]]}}]'
$DOTALL apply demo/financials.xlsx --all

# Wave 2: merge / unmerge on Revenue labels
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"merge_cells","payload":{"sheet":"Revenue","range":"A1:B1"}}]'
$DOTALL apply demo/financials.xlsx --all
$DOTALL read demo/financials.xlsx --selector-kind merges --selector Revenue
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"unmerge_cells","payload":{"sheet":"Revenue","range":"A1:B1"}}]'
$DOTALL apply demo/financials.xlsx --all

$DOTALL history demo/financials.xlsx
```

---

## PPTX — `deck.pptx`

Slide 1: title `Q3 Product Review`, body blurb, table `Metrics` (NPS=42), speaker notes.
Slide 2: `Next Steps`.

```bash
DOTALL=./target/debug/dotall

$DOTALL inspect demo/deck.pptx
$DOTALL read demo/deck.pptx --selector-kind slide --selector 'Slide 1'
$DOTALL read demo/deck.pptx --selector-kind notes --selector 'Slide 1'

# Prior v0
$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"set_shape_text","payload":{"slide":"Slide 1","shape":"Title","text":"Q3 Review (Updated)"}}]'
$DOTALL apply demo/deck.pptx --all

# Wave 1
$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"set_table_cell_text","payload":{"slide":"Slide 1","table":"Metrics","row":1,"col":1,"text":"58"}}]'
$DOTALL apply demo/deck.pptx --all

# Wave 2 — add blank slide after Slide 1, then delete the trailing Next Steps slide
$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"add_slide","payload":{"after":"Slide 1"}}]'
$DOTALL apply demo/deck.pptx --all
$DOTALL inspect demo/deck.pptx   # expect 3 slides
$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"delete_slide","payload":{"slide":"Slide 3"}}]'
$DOTALL apply demo/deck.pptx --all
$DOTALL inspect demo/deck.pptx   # expect 2 slides (updated title slide + blank)

# Wave 3 — speaker notes (notes part only)
$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"set_notes_text","payload":{"slide":"Slide 1","text":"Call out the NPS jump to 58."}}]'
$DOTALL apply demo/deck.pptx --all
$DOTALL read demo/deck.pptx --selector-kind notes --selector 'Slide 1'
```

Note: after `add_slide` after Slide 1, former “Next Steps” becomes Slide 3; deleting Slide 3 leaves the blank Slide 2. Notes stay on the original title slide part.

---

## DOCX — `memo.docx`

Body document-order indices: `0` heading, `1`–`2` body, `3`–`6` table cells
(`Owner`/`Status` header row, then `Platform` / `In progress`).

Header part `header1` index `0`: `CONFIDENTIAL - Agent Pilot Memo` (separate from body).

```bash
DOTALL=./target/debug/dotall

$DOTALL inspect demo/memo.docx
$DOTALL read demo/memo.docx --selector-kind paragraphs --selector '0:7'
$DOTALL read demo/memo.docx --selector-kind headers --selector '0:1'

# Prior v0 (body paragraph)
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"set_paragraph_text","payload":{"index":1,"text":"Pilot complete; expanding to PPTX and PDF."}}]'
$DOTALL apply demo/memo.docx --all

# Wave 1 (table cell)
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"set_paragraph_text","payload":{"index":6,"text":"Done"}}]'
$DOTALL apply demo/memo.docx --all

# Wave 2 (header)
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"set_header_paragraph_text","payload":{"part":"header1","index":0,"text":"CONFIDENTIAL - Updated"}}]'
$DOTALL apply demo/memo.docx --all
```

---

## PDF — `form.pdf`

Fields: `Name` (tx), `Email` (tx), `Agree` (btn checkbox, export `Yes`/`Off`),
`Department` (ch: Engineering / Sales / Operations).
`/Info`: Title `Vendor Intake Form`, Author `Dotall Demo`, Subject `Vendor onboarding`.

```bash
DOTALL=./target/debug/dotall

$DOTALL inspect demo/form.pdf
# summary.metadata.title / author / …; fields[].options for Department
$DOTALL read demo/form.pdf --selector-kind field --selector Department

# Prior v0
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field","payload":{"name":"Name","value":"Grace Hopper"}}]'
$DOTALL apply demo/form.pdf --all

# Wave 1
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field","payload":{"name":"Agree","value":"Yes"}}]'
$DOTALL apply demo/form.pdf --all

# Wave 2
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field","payload":{"name":"Department","value":"Sales"}}]'
$DOTALL apply demo/form.pdf --all
```

---

## MCP

1. Build: `cargo build --release -p dotall-mcp`
2. Copy [`mcp.example.json`](mcp.example.json); replace `REPO_ROOT`; keep `cwd` as `…/demo`.
3. Attach the skill for the format you are editing.
4. Example prompt:

   > Read `skills/xlsx/SKILL.md` and `skills/pptx/SKILL.md`. In this demo workspace,
   > inspect `financials.xlsx` and `deck.pptx`, bump Rate to 0.15, set the Metrics NPS
   > cell to 58, apply both, then show history.

## Tips

- Close or refresh Office apps after apply so you see new values.
- `.all/` under `demo/` is gitignored runtime state — safe to delete and regenerate.
- Prefer a **release** MCP binary for live demos; `cargo run` is slow.
- After regenerating demos, discard any leftover `.all/` history or re-init if hashes confuse you.
