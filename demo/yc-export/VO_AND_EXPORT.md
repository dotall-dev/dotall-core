# YC ~90s — VO script + export checklist

Record locally with Cursor MCP + Excel/LibreOffice. Kit is ready in `demo/`.

## Timed VO

**(0–12s)**  
Agents can edit code with search, patch, and undo. On spreadsheets they overwrite files and break them, with no undo.

**(12–28s)**  
This is a real multi-sheet SaaS model — hundreds of formulas across regions.

**(28–48s)**  
Stage a multi-cell, cross-sheet edit — then apply to the real file.

**(48–65s)**  
The file on disk changed. Cross-sheet KPIs updated. Chart and formulas preserved.

**(65–82s)**  
Revert restores the file surgically — reviewable, undoable.

**(82–90s)**  
.all — the layer that makes files agent-native. Excel now, every format next.

## Export checklist

- [ ] 1080p, ~90 seconds
- [ ] Changed-file Excel cut (Assumptions 59 / Regions Jan bumped / Board visible)
- [ ] Reverted-file Excel cut (back to 49 / original Jan seeds)
- [ ] Optional captions
- [ ] Keep `demo/yc-tape/mcp_raw_take.jsonl` as interview raw take (regenerate via MCP rehearsal if needed)

## Commands before rolling

```bash
cargo build --release -p dotall-cli -p dotall-mcp
./demo/yc_rehearse.sh          # optional confidence + cutaway xlsx stills
# Point MCP at demo/mcp.local.json (or mcp.example.json with REPO_ROOT filled)
# Paste prompt from demo/YC_VIDEO.md
```
