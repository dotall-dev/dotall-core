# Dotall demo workspace

Multi-format samples for CLI and MCP walkthroughs. Regenerate after format changes:

```bash
cargo run -p dotall-cli --example generate_demos
```

| File | Format | Agent workload |
|------|--------|----------------|
| [`financials.xlsx`](financials.xlsx) | XLSX | Multi-sheet formulas, named range `Rate`, merged header, percent/currency formats, seeded column widths + freeze panes at B2 |
| [`deck.pptx`](deck.pptx) | PPTX | Title + metrics table + speaker notes; second slide next-steps |
| [`memo.docx`](memo.docx) | DOCX | Heading memo + multi-run body + status table + confidential header |
| [`form.pdf`](form.pdf) | PDF | Intake form + page labels; Name/Email/Agree/Priority/Department; `/Info` metadata |

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
| `Inputs` | B2 | `0.10` (Rate) — named range `Rate`, number format `0%` |
| `Inputs` | B3 | `100` (Base) — number format `$#,##0.00` |
| `Revenue` | B2 / B3 | Jan / Feb amounts (`$#,##0.00`) |
| `Revenue` | B4 | `=B2+B3` (Total, currency) |
| `Revenue` | B5 | `=B4*Inputs!B2` (Commission, currency) |

```bash
DOTALL=./target/debug/dotall

$DOTALL inspect demo/financials.xlsx
# summary.style_table[] lists style_ids; cells carry number_format via ast_range
$DOTALL read demo/financials.xlsx --selector-kind named_ranges
$DOTALL read demo/financials.xlsx --selector-kind merges --selector Inputs
$DOTALL read demo/financials.xlsx --range 'Revenue!A1:B5'
$DOTALL read demo/financials.xlsx --selector-kind ast_range --selector 'Inputs!B2:B3'
# expect B2 number_format "0%", B3 "$#,##0.00", plus style_id
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

# Wave 4: column width / row height (worksheet XML only)
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"set_column_width","payload":{"sheet":"Inputs","column":"A","width":22.5}}]'
$DOTALL apply demo/financials.xlsx --all
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"set_row_height","payload":{"sheet":"Inputs","row":1,"height":36}}]'
$DOTALL apply demo/financials.xlsx --all

# Wave 5: freeze_panes (sheet view); inspect shows freeze_panes
$DOTALL inspect demo/financials.xlsx   # Inputs freeze_panes: B2 (seeded)
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"freeze_panes","payload":{"sheet":"Inputs","cell":"A2"}}]'
$DOTALL apply demo/financials.xlsx --all
$DOTALL inspect demo/financials.xlsx   # expect freeze_panes A2
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"freeze_panes","payload":{"sheet":"Inputs","cell":"B2"}}]'
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

# Wave 4 — reorder: move blank Slide 2 to front (title+notes become Slide 2)
$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"move_slide","payload":{"slide":"Slide 2","to_index":0}}]'
$DOTALL apply demo/deck.pptx --all
$DOTALL inspect demo/deck.pptx   # expect blank first, then title slide
$DOTALL read demo/deck.pptx --selector-kind notes --selector 'Slide 2'

# Wave 5 — add a text box on the title slide (now Slide 2 after reorder)
$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"add_textbox","payload":{"slide":"Slide 2","name":"Callout","text":"Agent follow-up"}}]'
$DOTALL apply demo/deck.pptx --all
$DOTALL read demo/deck.pptx --selector-kind slide --selector 'Slide 2'
```

Note: after `add_slide` after Slide 1, former “Next Steps” becomes Slide 3; deleting Slide 3 leaves the blank Slide 2. Notes stay on the original title slide part. `move_slide` only rewrites `presentation.xml` order — slide/notes parts stay byte-identical. `add_textbox` patches only the target slide part.

---

## DOCX — `memo.docx`

Body document-order indices: `0` heading, `1`–`2` body, `3`–`6` table cells
(`Owner`/`Status` header row, then `Platform` / `In progress`).

Paragraph `1` is multi-run (bold + italic) for Wave 3 fidelity smoke.

Header part `header1` index `0`: `CONFIDENTIAL - Agent Pilot Memo` (separate from body).

```bash
DOTALL=./target/debug/dotall

$DOTALL inspect demo/memo.docx
$DOTALL read demo/memo.docx --selector-kind paragraphs --selector '0:7'
$DOTALL read demo/memo.docx --selector-kind headers --selector '0:1'

# Wave 3 (multi-run: preserve bold + italic rPr per run) — run on fresh memo
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"set_paragraph_text","payload":{"index":1,"runs":[{"text":"Pilot complete; "},{"text":"expanding to PPTX and PDF."}]}}]'
$DOTALL apply demo/memo.docx --all

# Prior v0 (plain text — keeps first-run rPr, clears subsequent runs)
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"set_paragraph_text","payload":{"index":2,"text":"Please update the status table below before Friday standup."}}]'
$DOTALL apply demo/memo.docx --all

# Wave 1 (table cell)
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"set_paragraph_text","payload":{"index":6,"text":"Done"}}]'
$DOTALL apply demo/memo.docx --all

# Wave 2 (header)
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"set_header_paragraph_text","payload":{"part":"header1","index":0,"text":"CONFIDENTIAL - Updated"}}]'
$DOTALL apply demo/memo.docx --all

# Wave 4 — insert after body paragraph 2 (before the status table); later indices shift +1
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"insert_paragraph","payload":{"after":2,"text":"Action: confirm owners before Friday."}}]'
$DOTALL apply demo/memo.docx --all
$DOTALL read demo/memo.docx --selector-kind paragraphs --selector '2:5'
# expect index 3 = Action: confirm owners before Friday.

# Wave 5 — delete the inserted action paragraph (index 3 after insert); later indices shift -1
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"delete_paragraph","payload":{"index":3}}]'
$DOTALL apply demo/memo.docx --all
$DOTALL read demo/memo.docx --selector-kind paragraphs --selector '2:5'
# expect no "Action: confirm owners…" paragraph; table cells resume at index 3.
```

---

## PDF — `form.pdf`

Fields: `Name` (tx), `Email` (tx), `Agree` (btn checkbox, export `Yes`/`Off`),
`Priority` (btn radio, export `Low`/`Medium`/`High`/`Off`, default Medium),
`Department` (ch: Engineering / Sales / Operations).
`/Info`: Title `Vendor Intake Form`, Author `Dotall Demo`, Subject `Vendor onboarding`.
Page text (labels): Vendor Intake Form, Name, Email, Agree to terms, Priority, Department.

```bash
DOTALL=./target/debug/dotall

$DOTALL inspect demo/form.pdf
# summary.metadata.title / author / …; fields[].options for Department;
# fields[].export_values for Agree and Priority
$DOTALL read demo/form.pdf --selector-kind page --selector 1
# Vendor Intake Form / Name / Email / Agree to terms / Priority / Department
$DOTALL read demo/form.pdf --selector-kind field --selector Priority

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

# Wave 4 — /Info Title/Author/Subject (inspect metadata)
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_document_metadata","payload":{"title":"Updated Intake","author":"Wave 4 Agent","subject":"Onboarding refresh"}}]'
$DOTALL apply demo/form.pdf --all
$DOTALL inspect demo/form.pdf
# expect summary.metadata.title Updated Intake, author Wave 4 Agent

# Wave 5 — radio group via explicit export value
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field","payload":{"name":"Priority","value":"High"}}]'
$DOTALL apply demo/form.pdf --all
$DOTALL read demo/form.pdf --selector-kind field --selector Priority
# expect value High
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
