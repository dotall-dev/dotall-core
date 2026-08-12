# YC demo — Cursor in `yc_demo/`

**Open this folder as the Cursor workspace:** `/workspace/yc_demo`

That folder has only `saas_model.xlsx` and `.cursor/mcp.json`.

## Before each take

From the repo root:

```bash
cargo build --release -p dotall-mcp
cargo run -q -p dotall-xlsx --example gen_yc_demo_workbook
cp demo/saas_model.xlsx yc_demo/saas_model.xlsx
rm -rf yc_demo/.all && ./target/release/dotall init yc_demo
```

Reload Cursor; confirm **dotall** is green under Tools & MCP.

## Recording

| Step | Action |
|------|--------|
| 1 | Show `saas_model.xlsx` in Excel (Assumptions / Board) |
| 2 | Paste **Prompt 1** from [`PROMPTS.md`](PROMPTS.md) |
| 3 | Reopen Excel → edited values |
| 4 | Paste **Prompt 2** |
| 5 | Reopen Excel → reverted |

Prompts are natural language — the agent should discover Dotall MCP tools on its own (no skill file).
