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

Heavy Q3 board-pack bake-off fixtures live under [`q3-pack/`](q3-pack/README.md)
(`q3-financials.xlsx`, `q3-deck.pptx`, `q3-memo.docx`, `q3-intake.pdf`) — the
Dotall vs no-Dotall investor demo. See `q3-pack/README.md` for the brief,
expected live targets, decoy warning, and filming steps.

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

# Wave 6: define_name (workbook.xml only; worksheets stay byte-identical)
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"define_name","payload":{"name":"Rate","formula":"Inputs!$B$3"}}]'
$DOTALL apply demo/financials.xlsx --all
$DOTALL read demo/financials.xlsx --selector-kind named_ranges
# expect Rate → Inputs!$B$3
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"define_name","payload":{"name":"Rate","formula":"Inputs!$B$2"}}]'
$DOTALL apply demo/financials.xlsx --all

# Wave 7: delete_name (workbook.xml only), then restore Rate for demos
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"delete_name","payload":{"name":"Rate"}}]'
$DOTALL apply demo/financials.xlsx --all
$DOTALL read demo/financials.xlsx --selector-kind named_ranges
# expect Rate gone
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"define_name","payload":{"name":"Rate","formula":"Inputs!$B$2"}}]'
$DOTALL apply demo/financials.xlsx --all

# Wave 8: hide_sheet (workbook.xml only), then unhide for demos
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"hide_sheet","payload":{"sheet":"Revenue","hidden":true}}]'
$DOTALL apply demo/financials.xlsx --all
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"hide_sheet","payload":{"sheet":"Revenue","hidden":false}}]'
$DOTALL apply demo/financials.xlsx --all

# Wave 9: set_tab_color on Inputs (sheetPr/tabColor), then clear
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"set_tab_color","payload":{"sheet":"Inputs","color":"FF4472C4"}}]'
$DOTALL apply demo/financials.xlsx --all
$DOTALL inspect demo/financials.xlsx
# expect sheets[].tab_color FF4472C4 for Inputs
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"set_tab_color","payload":{"sheet":"Inputs","color":null}}]'
$DOTALL apply demo/financials.xlsx --all

# Wave 10: set_auto_filter on Revenue, then clear
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"set_auto_filter","payload":{"sheet":"Revenue","range":"A1:B5"}}]'
$DOTALL apply demo/financials.xlsx --all
$DOTALL inspect demo/financials.xlsx
# expect sheets[].auto_filter A1:B5 for Revenue
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"set_auto_filter","payload":{"sheet":"Revenue","range":null}}]'
$DOTALL apply demo/financials.xlsx --all

# Wave 11: set_print_area on Revenue, then clear
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"set_print_area","payload":{"sheet":"Revenue","range":"A1:B5"}}]'
$DOTALL apply demo/financials.xlsx --all
$DOTALL inspect demo/financials.xlsx
# expect sheets[].print_area A1:B5 for Revenue
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"set_print_area","payload":{"sheet":"Revenue","range":null}}]'
$DOTALL apply demo/financials.xlsx --all

# Wave 12: set_print_titles (repeat row 1 + col A), then clear
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"set_print_titles","payload":{"sheet":"Revenue","rows":"1:1","cols":"A:A"}}]'
$DOTALL apply demo/financials.xlsx --all
$DOTALL inspect demo/financials.xlsx
# expect sheets[].print_titles rows=1:1 cols=A:A for Revenue
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"set_print_titles","payload":{"sheet":"Revenue","rows":null,"cols":null}}]'
$DOTALL apply demo/financials.xlsx --all

# Wave 13: set_page_orientation landscape, then portrait
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"set_page_orientation","payload":{"sheet":"Revenue","orientation":"landscape"}}]'
$DOTALL apply demo/financials.xlsx --all
$DOTALL inspect demo/financials.xlsx
# expect sheets[].page_orientation landscape for Revenue
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"set_page_orientation","payload":{"sheet":"Revenue","orientation":"portrait"}}]'
$DOTALL apply demo/financials.xlsx --all

# Wave 14: set_page_margins (inches)
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"set_page_margins","payload":{"sheet":"Revenue","left":0.5,"right":0.5,"top":0.6,"bottom":0.6,"header":0.25,"footer":0.25}}]'
$DOTALL apply demo/financials.xlsx --all
$DOTALL inspect demo/financials.xlsx
# expect sheets[].page_margins left=0.5 for Revenue

# Wave 15: set_print_scale (pageSetup scale percent)
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"set_print_scale","payload":{"sheet":"Revenue","scale":75}}]'
$DOTALL apply demo/financials.xlsx --all
$DOTALL inspect demo/financials.xlsx
# expect sheets[].print_scale 75 for Revenue

# Wave 16: set_fit_to_page (fitToWidth/Height + pageSetUpPr)
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"set_fit_to_page","payload":{"sheet":"Revenue","width":1,"height":1}}]'
$DOTALL apply demo/financials.xlsx --all
$DOTALL inspect demo/financials.xlsx
# expect sheets[].fit_to_page width=1 height=1 for Revenue
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"set_fit_to_page","payload":{"sheet":"Revenue","width":null,"height":null}}]'
$DOTALL apply demo/financials.xlsx --all

# Wave 17: set_center_on_page (printOptions horizontal/verticalCentered)
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"set_center_on_page","payload":{"sheet":"Revenue","horizontal":true,"vertical":false}}]'
$DOTALL apply demo/financials.xlsx --all
$DOTALL inspect demo/financials.xlsx
# expect sheets[].center_on_page horizontal=true vertical=false for Revenue
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"set_center_on_page","payload":{"sheet":"Revenue","horizontal":false,"vertical":false}}]'
$DOTALL apply demo/financials.xlsx --all

# Wave 18: set_paper_size (pageSetup paperSize; 9=A4)
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"set_paper_size","payload":{"sheet":"Revenue","paper_size":9}}]'
$DOTALL apply demo/financials.xlsx --all
$DOTALL inspect demo/financials.xlsx
# expect sheets[].paper_size 9 for Revenue

# Wave 19: set_header_footer (oddHeader/oddFooter; Excel codes passed through)
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"set_header_footer","payload":{"sheet":"Revenue","header":"&CBoard pack","footer":"&P"}}]'
$DOTALL apply demo/financials.xlsx --all
$DOTALL inspect demo/financials.xlsx
# expect sheets[].header_footer header=&CBoard pack footer=&P for Revenue

# Wave 20: set_sheet_zoom (sheetView zoomScale)
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"set_sheet_zoom","payload":{"sheet":"Revenue","zoom":75}}]'
$DOTALL apply demo/financials.xlsx --all
$DOTALL inspect demo/financials.xlsx
# expect sheets[].zoom 75 for Revenue

# Wave 21: hide gridlines (sheetView showGridLines=0)
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"set_show_gridlines","payload":{"sheet":"Revenue","show":false}}]'
$DOTALL apply demo/financials.xlsx --all
$DOTALL inspect demo/financials.xlsx
# expect sheets[].show_gridlines false for Revenue

# Wave 22: set_right_to_left (sheetView rightToLeft)
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"set_right_to_left","payload":{"sheet":"Revenue","rtl":true}}]'
$DOTALL apply demo/financials.xlsx --all
$DOTALL inspect demo/financials.xlsx
# expect sheets[].right_to_left true for Revenue

# Wave 23: insert_comment (legacy Excel Note: comments.xml + VML)
$DOTALL inspect demo/financials.xlsx
# expect summary.comments[] (empty) and summary.charts[]
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"insert_comment","payload":{"sheet":"Inputs","address":"B2","text":"Review Rate","author":"Dotall"}}]'
$DOTALL apply demo/financials.xlsx --all
$DOTALL inspect demo/financials.xlsx
# expect comments[] sheet=Inputs cell=B2 text=Review Rate (Excel Review → Notes)

# Wave 24: insert_picture (new drawing + xl/media; existing media byte-identical)
$DOTALL inspect demo/financials.xlsx
# expect summary.pictures[]
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"insert_picture","payload":{"sheet":"Inputs","from_cell":"C1","bytes_base64":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVQI12P4z8AAAAMBAQAY3Y20AAAAAElFTkSuQmCC","content_type":"image/png"}}]'
$DOTALL apply demo/financials.xlsx --all
$DOTALL inspect demo/financials.xlsx
# expect pictures[] sheet=Inputs from_cell=C1

# Wave 25: set_cell_font + set_cell_fill (surgical styles.xml + cell s=)
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"set_cell_font","payload":{"sheet":"Inputs","address":"A1","bold":true,"size_pt":14,"color":"#1F4E79"}}]'
$DOTALL apply demo/financials.xlsx --all
$DOTALL edit demo/financials.xlsx --ops-json \
  '[{"kind":"set_cell_fill","payload":{"sheet":"Inputs","address":"A1","color":"#FFFF00"}}]'
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

# Wave 6 — delete the Callout text box (p:sp only; tables stay)
$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"delete_shape","payload":{"slide":"Slide 2","shape":"Callout"}}]'
$DOTALL apply demo/deck.pptx --all
$DOTALL read demo/deck.pptx --selector-kind slide --selector 'Slide 2'
# expect Callout gone; Title / Metrics remain

# Wave 7 — rename Title → Headline on the title slide (Slide 2 after reorder)
$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"rename_shape","payload":{"slide":"Slide 2","shape":"Title","name":"Headline"}}]'
$DOTALL apply demo/deck.pptx --all
$DOTALL read demo/deck.pptx --selector-kind slide --selector 'Slide 2'
# expect Headline shape; restore for demos:
$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"rename_shape","payload":{"slide":"Slide 2","shape":"Headline","name":"Title"}}]'
$DOTALL apply demo/deck.pptx --all

# Wave 8 — bold Title text on the title slide (content-on-slide)
$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"set_shape_bold","payload":{"slide":"Slide 2","shape":"Title","bold":true}}]'
$DOTALL apply demo/deck.pptx --all

# Wave 9 — italic Title text (content-on-slide companion to bold)
$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"set_shape_italic","payload":{"slide":"Slide 2","shape":"Title","italic":true}}]'
$DOTALL apply demo/deck.pptx --all

# Wave 10 — underline Title text (content-on-slide companion to bold/italic)
$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"set_shape_underline","payload":{"slide":"Slide 2","shape":"Title","underline":true}}]'
$DOTALL apply demo/deck.pptx --all

# Wave 11 — font size on Title (content-on-slide companion to bold/italic/underline)
$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"set_shape_font_size","payload":{"slide":"Slide 2","shape":"Title","size_pt":28}}]'
$DOTALL apply demo/deck.pptx --all

# Wave 12 — font name on Title (content-on-slide companion to font size)
$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"set_shape_font_name","payload":{"slide":"Slide 2","shape":"Title","font":"Arial"}}]'
$DOTALL apply demo/deck.pptx --all

# Wave 13 — font color on Title (content-on-slide companion to font name)
$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"set_shape_font_color","payload":{"slide":"Slide 2","shape":"Title","color":"#FF0000"}}]'
$DOTALL apply demo/deck.pptx --all

# Wave 14 — highlight on Title (content-on-slide companion to font color)
$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"set_shape_highlight","payload":{"slide":"Slide 2","shape":"Title","color":"#FFFF00"}}]'
$DOTALL apply demo/deck.pptx --all

# Wave 15 — find/replace inside Title text (content-on-slide)
$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"replace_shape_text","payload":{"slide":"Slide 1","shape":"Title","find":"Q3","replace":"Q4"}}]'
$DOTALL apply demo/deck.pptx --all
$DOTALL read demo/deck.pptx --selector-kind slide --selector 'Slide 1'
# expect Title contains Q4

# Wave 16 — strikethrough on Title (content-on-slide)
$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"set_shape_strikethrough","payload":{"slide":"Slide 2","shape":"Title","strikethrough":true}}]'
$DOTALL apply demo/deck.pptx --all

# Wave 17 — superscript on Title (a:rPr baseline)
$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"set_shape_vert_align","payload":{"slide":"Slide 2","shape":"Title","vert_align":"superscript"}}]'
$DOTALL apply demo/deck.pptx --all

# Wave 18 — small caps on Title (a:rPr cap)
$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"set_shape_caps","payload":{"slide":"Slide 2","shape":"Title","caps":"small"}}]'
$DOTALL apply demo/deck.pptx --all

# Wave 19 — hyperlink on Title (a:hlinkClick + slide .rels)
$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"set_shape_hyperlink","payload":{"slide":"Slide 2","shape":"Title","url":"https://example.com"}}]'
$DOTALL apply demo/deck.pptx --all

# Wave 20 — find/replace across text frames on Slide 2
$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"replace_across_shapes","payload":{"slide":"Slide 2","find":"Next","replace":"Follow-up"}}]'
$DOTALL apply demo/deck.pptx --all

# Wave 21 — bullets on Title (a:buChar)
$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"set_shape_bullet","payload":{"slide":"Slide 2","shape":"Title","bullet":true}}]'
$DOTALL apply demo/deck.pptx --all

# Wave 22 — bold Metrics cell (a:rPr b)
$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"set_table_cell_bold","payload":{"slide":"Slide 1","table":"Metrics","row":1,"col":1,"bold":true}}]'
$DOTALL apply demo/deck.pptx --all

# Wave 23 — insert_comment (classic ISO commentAuthors + comments/commentN.xml)
$DOTALL inspect demo/deck.pptx
# expect summary.comments[] and summary.charts[]
$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"insert_comment","payload":{"slide":"Slide 1","text":"Check KPI","author":"Dotall"}}]'
$DOTALL apply demo/deck.pptx --all
$DOTALL inspect demo/deck.pptx
# expect comments[] slide=Slide 1 text=Check KPI

# Wave 24 — insert_picture (new p:pic + ppt/media; existing media byte-identical)
$DOTALL inspect demo/deck.pptx
# expect summary.pictures[]
$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"insert_picture","payload":{"slide":"Slide 1","name":"Logo","bytes_base64":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVQI12P4z8AAAAMBAQAY3Y20AAAAAElFTkSuQmCC","content_type":"image/png"}}]'
$DOTALL apply demo/deck.pptx --all
$DOTALL inspect demo/deck.pptx
# expect pictures[] slide=Slide 1 name=Logo

# Wave 25 — italic Metrics cell (a:rPr i; not cell fill)
$DOTALL edit demo/deck.pptx --ops-json \
  '[{"kind":"set_table_cell_italic","payload":{"slide":"Slide 1","table":"Metrics","row":1,"col":1,"italic":true}}]'
$DOTALL apply demo/deck.pptx --all
```

Note: after `add_slide` after Slide 1, former “Next Steps” becomes Slide 3; deleting Slide 3 leaves the blank Slide 2. Notes stay on the original title slide part. `move_slide` only rewrites `presentation.xml` order — slide/notes parts stay byte-identical. `add_textbox` / `delete_shape` / `rename_shape` / `set_shape_bold` / `set_shape_italic` / `set_shape_underline` / `set_shape_font_size` / `set_shape_font_name` / `set_shape_font_color` / `set_shape_highlight` / `set_shape_strikethrough` / `set_shape_vert_align` / `set_shape_caps` / `replace_shape_text` / `replace_across_shapes` / `set_shape_bullet` / `set_table_cell_text` / `set_table_cell_bold` / `set_table_cell_italic` patch only the target slide part. `set_shape_hyperlink` also patches that slide’s `.rels`.

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

# Wave 6 — set paragraph style (w:pStyle only; styles.xml untouched)
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"set_paragraph_style","payload":{"index":2,"style_id":"Heading1"}}]'
$DOTALL apply demo/memo.docx --all
$DOTALL read demo/memo.docx --selector-kind paragraphs --selector '2:3'
# expect index 2 style_id Heading1

# Wave 7 — paragraph alignment (w:jc)
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"set_paragraph_alignment","payload":{"index":2,"alignment":"center"}}]'
$DOTALL apply demo/memo.docx --all

# Wave 8 — paragraph bold (w:b on runs)
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"set_paragraph_bold","payload":{"index":2,"bold":true}}]'
$DOTALL apply demo/memo.docx --all

# Wave 9 — paragraph italic (w:i on runs)
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"set_paragraph_italic","payload":{"index":2,"italic":true}}]'
$DOTALL apply demo/memo.docx --all

# Wave 10 — paragraph underline (w:u on runs)
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"set_paragraph_underline","payload":{"index":2,"underline":true}}]'
$DOTALL apply demo/memo.docx --all

# Wave 11 — paragraph font size (w:sz / w:szCs from size_pt)
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"set_paragraph_font_size","payload":{"index":2,"size_pt":14}}]'
$DOTALL apply demo/memo.docx --all

# Wave 12 — paragraph font name (w:rFonts)
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"set_paragraph_font_name","payload":{"index":2,"font":"Arial"}}]'
$DOTALL apply demo/memo.docx --all

# Wave 13 — paragraph font color (w:color)
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"set_paragraph_font_color","payload":{"index":2,"color":"#C00000"}}]'
$DOTALL apply demo/memo.docx --all

# Wave 14 — paragraph highlight (w:highlight)
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"set_paragraph_highlight","payload":{"index":2,"color":"yellow"}}]'
$DOTALL apply demo/memo.docx --all

# Wave 15 — find/replace inside a body paragraph
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"replace_paragraph_text","payload":{"index":2,"find":"Friday","replace":"Monday"}}]'
$DOTALL apply demo/memo.docx --all
$DOTALL read demo/memo.docx --selector-kind paragraphs --selector '2:3'
# expect paragraph 2 mentions Monday

# Wave 16 — paragraph strikethrough (w:strike)
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"set_paragraph_strikethrough","payload":{"index":2,"strikethrough":true}}]'
$DOTALL apply demo/memo.docx --all

# Wave 17 — paragraph superscript (w:vertAlign)
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"set_paragraph_vert_align","payload":{"index":2,"vert_align":"superscript"}}]'
$DOTALL apply demo/memo.docx --all

# Wave 18 — paragraph small caps (w:smallCaps)
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"set_paragraph_caps","payload":{"index":2,"caps":"small"}}]'
$DOTALL apply demo/memo.docx --all

# Wave 19 — paragraph hyperlink (w:hyperlink + document.xml.rels)
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"set_paragraph_hyperlink","payload":{"index":2,"url":"https://example.com"}}]'
$DOTALL apply demo/memo.docx --all

# Wave 20 — find/replace across editable body/table paragraphs
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"replace_across_paragraphs","payload":{"find":"Monday","replace":"Tuesday"}}]'
$DOTALL apply demo/memo.docx --all

# Wave 21 — paragraph bullet (w:numPr + numbering.xml)
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"set_paragraph_bullet","payload":{"index":2,"bullet":true}}]'
$DOTALL apply demo/memo.docx --all

# Wave 22 — table cell shading (w:shd fill on enclosing w:tc)
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"set_cell_shading","payload":{"index":3,"color":"#FFFF00"}}]'
$DOTALL apply demo/memo.docx --all

# Wave 23 — insert_comment (word/comments.xml + commentRange on a paragraph)
$DOTALL inspect demo/memo.docx
# expect summary.comments[] and summary.charts []
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"insert_comment","payload":{"index":1,"text":"Confirm owners","author":"Dotall"}}]'
$DOTALL apply demo/memo.docx --all
$DOTALL inspect demo/memo.docx
# expect comments[] index=1 text=Confirm owners

# Wave 24 — insert_picture (inline drawing + word/media)
$DOTALL inspect demo/memo.docx
# expect summary.pictures[]
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"insert_picture","payload":{"index":1,"bytes_base64":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVQI12P4z8AAAAMBAQAY3Y20AAAAAElFTkSuQmCC","content_type":"image/png"}}]'
$DOTALL apply demo/memo.docx --all
$DOTALL inspect demo/memo.docx
# expect pictures[] index=1

# Wave 25 — paragraph spacing (twips) and page break
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"set_paragraph_spacing","payload":{"index":1,"before_pt":12,"after_pt":6}}]'
$DOTALL apply demo/memo.docx --all
$DOTALL edit demo/memo.docx --ops-json \
  '[{"kind":"insert_page_break","payload":{"index":2}}]'
$DOTALL apply demo/memo.docx --all
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

# Wave 6 — clear_form_field (blank Tx/Ch, Off for Btn)
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"clear_form_field","payload":{"name":"Name"}}]'
$DOTALL apply demo/form.pdf --all
$DOTALL read demo/form.pdf --selector-kind field --selector Name
# expect empty value
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"clear_form_field","payload":{"name":"Agree"}}]'
$DOTALL apply demo/form.pdf --all
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"clear_form_field","payload":{"name":"Department"}}]'
$DOTALL apply demo/form.pdf --all
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"clear_form_field","payload":{"name":"Priority"}}]'
$DOTALL apply demo/form.pdf --all
$DOTALL read demo/form.pdf --selector-kind field --selector Priority
# expect value Off

# Wave 7 — clear /Info Title/Author/Subject (Creator/Producer remain)
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"clear_document_metadata","payload":{}}]'
$DOTALL apply demo/form.pdf --all
$DOTALL inspect demo/form.pdf
# expect summary.metadata title/author/subject empty or absent
# restore for demos:
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_document_metadata","payload":{"title":"Vendor Intake Form","author":"Dotall Demo","subject":"Vendor onboarding"}}]'
$DOTALL apply demo/form.pdf --all

# Wave 8 — clear every editable AcroForm field
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"clear_all_form_fields","payload":{}}]'
$DOTALL apply demo/form.pdf --all
$DOTALL read demo/form.pdf --selector-kind field --selector Name
# expect blank Name; Agree/Priority Off

# Wave 9 — bulk set multiple fields in one transaction
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_fields","payload":{"fields":{"Name":"Grace Hopper","Agree":"On","Department":"Engineering"}}}]'
$DOTALL apply demo/form.pdf --all
$DOTALL read demo/form.pdf --selector-kind field --selector Name
# expect Grace Hopper; Agree Yes

# Wave 10 — lock Name (Ff ReadOnly), then unlock
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field_readonly","payload":{"name":"Name","readonly":true}}]'
$DOTALL apply demo/form.pdf --all
$DOTALL inspect demo/form.pdf
# expect fields[].read_only true for Name
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field_readonly","payload":{"name":"Name","readonly":false}}]'
$DOTALL apply demo/form.pdf --all

# Wave 11 — mark Name required (Ff Required), then clear
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field_required","payload":{"name":"Name","required":true}}]'
$DOTALL apply demo/form.pdf --all
$DOTALL inspect demo/form.pdf
# expect fields[].required true for Name
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field_required","payload":{"name":"Name","required":false}}]'
$DOTALL apply demo/form.pdf --all

# Wave 12 — mark Name multiline (Ff Multiline), then clear
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field_multiline","payload":{"name":"Name","multiline":true}}]'
$DOTALL apply demo/form.pdf --all
$DOTALL inspect demo/form.pdf
# expect fields[].multiline true for Name
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field_multiline","payload":{"name":"Name","multiline":false}}]'
$DOTALL apply demo/form.pdf --all

# Wave 13 — mark Name password (Ff Password), then clear
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field_password","payload":{"name":"Name","password":true}}]'
$DOTALL apply demo/form.pdf --all
$DOTALL inspect demo/form.pdf
# expect fields[].password true for Name
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field_password","payload":{"name":"Name","password":false}}]'
$DOTALL apply demo/form.pdf --all

# Wave 14 — set Name MaxLen, then clear
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field_max_length","payload":{"name":"Name","max_length":32}}]'
$DOTALL apply demo/form.pdf --all
$DOTALL inspect demo/form.pdf
# expect fields[].max_length 32 for Name
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field_max_length","payload":{"name":"Name","max_length":null}}]'
$DOTALL apply demo/form.pdf --all

# Wave 15 — mark Name comb (Ff Comb), then clear
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field_comb","payload":{"name":"Name","comb":true}}]'
$DOTALL apply demo/form.pdf --all
$DOTALL inspect demo/form.pdf
# expect fields[].comb true for Name
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field_comb","payload":{"name":"Name","comb":false}}]'
$DOTALL apply demo/form.pdf --all

# Wave 16 — mark Name do-not-scroll (Ff DoNotScroll), then clear
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field_do_not_scroll","payload":{"name":"Name","do_not_scroll":true}}]'
$DOTALL apply demo/form.pdf --all
$DOTALL inspect demo/form.pdf
# expect fields[].do_not_scroll true for Name
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field_do_not_scroll","payload":{"name":"Name","do_not_scroll":false}}]'
$DOTALL apply demo/form.pdf --all

# Wave 17 — mark Name do-not-spell-check (Ff DoNotSpellCheck), then clear
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field_do_not_spell_check","payload":{"name":"Name","do_not_spell_check":true}}]'
$DOTALL apply demo/form.pdf --all
$DOTALL inspect demo/form.pdf
# expect fields[].do_not_spell_check true for Name
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field_do_not_spell_check","payload":{"name":"Name","do_not_spell_check":false}}]'
$DOTALL apply demo/form.pdf --all

# Wave 18 — mark Name rich-text (Ff RichText), then clear
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field_rich_text","payload":{"name":"Name","rich_text":true}}]'
$DOTALL apply demo/form.pdf --all
$DOTALL inspect demo/form.pdf
# expect fields[].rich_text true for Name
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field_rich_text","payload":{"name":"Name","rich_text":false}}]'
$DOTALL apply demo/form.pdf --all

# Wave 19 — mark Name no-export (Ff NoExport), then clear
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field_no_export","payload":{"name":"Name","no_export":true}}]'
$DOTALL apply demo/form.pdf --all
$DOTALL inspect demo/form.pdf
# expect fields[].no_export true for Name
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field_no_export","payload":{"name":"Name","no_export":false}}]'
$DOTALL apply demo/form.pdf --all

# Wave 20 — mark Department multi-select (Ff MultiSelect), then clear
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field_multi_select","payload":{"name":"Department","multi_select":true}}]'
$DOTALL apply demo/form.pdf --all
$DOTALL inspect demo/form.pdf
# expect fields[].multi_select true for Department
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field_multi_select","payload":{"name":"Department","multi_select":false}}]'
$DOTALL apply demo/form.pdf --all

# Wave 21 — mark Department combo (Ff Combo), then clear
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field_combo","payload":{"name":"Department","combo":true}}]'
$DOTALL apply demo/form.pdf --all
$DOTALL inspect demo/form.pdf
# expect fields[].combo true for Department
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field_combo","payload":{"name":"Department","combo":false}}]'
$DOTALL apply demo/form.pdf --all

# Wave 22 — mark Department edit (Ff Edit / combo type-in), then clear
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field_edit","payload":{"name":"Department","edit":true}}]'
$DOTALL apply demo/form.pdf --all
$DOTALL inspect demo/form.pdf
# expect fields[].edit true for Department
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"set_form_field_edit","payload":{"name":"Department","edit":false}}]'
$DOTALL apply demo/form.pdf --all

# Wave 23 — insert_comment (sticky /Text annotation; not a content-stream rewrite)
$DOTALL inspect demo/form.pdf
# expect summary.comments[] (Widget fields are not comments) and summary.charts []
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"insert_comment","payload":{"page":1,"contents":"Check Name field","author":"Dotall"}}]'
$DOTALL apply demo/form.pdf --all
$DOTALL inspect demo/form.pdf
# expect comments[] page=1 subtype=Text contents=Check Name field

# Wave 24 — insert_picture (stamp annotation; not a page content-stream rewrite)
$DOTALL inspect demo/form.pdf
# expect summary.pictures[] (Stamp annots, not Widget)
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"insert_picture","payload":{"page":1,"bytes_base64":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVQI12P4z8AAAAMBAQAY3Y20AAAAAElFTkSuQmCC","content_type":"image/png"}}]'
$DOTALL apply demo/form.pdf --all
$DOTALL inspect demo/form.pdf
# expect pictures[] page=1 subtype=Stamp

# Wave 25 — rotate_page (page /Rotate; not a content-stream redraw)
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"rotate_page","payload":{"page":1,"degrees":90}}]'
$DOTALL apply demo/form.pdf --all
$DOTALL inspect demo/form.pdf
# expect summary.page_rotations page=1 rotate=90
$DOTALL edit demo/form.pdf --ops-json \
  '[{"kind":"rotate_page","payload":{"page":1,"degrees":0}}]'
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
