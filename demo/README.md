# Dotall demo workspace

Sample spreadsheet for a 3–5 minute walkthrough of Dotall CLI and MCP.

## Workbook: `financials.xlsx`

| Sheet | Cell | Value / formula |
|-------|------|-----------------|
| `Inputs` | B2 | `0.10` (Rate) |
| `Inputs` | B3 | `100` (Base) |
| `Revenue` | B2 | `100` (Jan) |
| `Revenue` | B3 | `150` (Feb) |
| `Revenue` | B4 | `=B2+B3` (Total) |
| `Revenue` | B5 | `=B4*Inputs!B2` (Commission) |

Dependency story: changing **Rate** (`Inputs!B2`) affects **Commission** (`Revenue!B5`) via **Total** (`Revenue!B4`).

## Setup

From the repo root:

```bash
cargo build --release -p dotall-cli -p dotall-mcp

# Optional: warm .all/ (gitignored)
./target/release/dotall init demo
./target/release/dotall inspect demo/financials.xlsx
```

## CLI walkthrough (Story A)

```bash
DOTALL=./target/release/dotall
cd demo   # or use paths relative to repo root

$DOTALL inspect financials.xlsx
$DOTALL read financials.xlsx --range 'Revenue!A1:B5'
$DOTALL deps financials.xlsx --cell 'Revenue!B5'

# Stage only — Excel file unchanged until apply
$DOTALL edit financials.xlsx \
  --op set_cell_value --sheet Inputs --address B2 --value 0.15

$DOTALL staged financials.xlsx
$DOTALL apply financials.xlsx --all

$DOTALL history financials.xlsx
$DOTALL diff financials.xlsx --version 1

# Undo: revert stages a restore; apply commits it
$DOTALL revert financials.xlsx --version 1
$DOTALL apply financials.xlsx --all
```

Expected after apply: Rate is `0.15`. After revert+apply: Rate is `0.10` again.

## MCP walkthrough

1. Build the release binary:

   ```bash
   cargo build --release -p dotall-mcp
   ```

2. Copy [`mcp.example.json`](mcp.example.json) into your MCP client config.
   Replace `REPO_ROOT` with the absolute path to this repository.
   Set `cwd` to `…/demo` so tools resolve `financials.xlsx`.

3. Attach [`skills/xlsx/SKILL.md`](../skills/xlsx/SKILL.md) for the agent workflow.

4. Paste this prompt:

   > Read `skills/xlsx/SKILL.md`. In this workspace, inspect `financials.xlsx`,
   > show formula dependents of `Revenue!B5`, change `Inputs!B2` from 0.1 to 0.15,
   > stage the edit, apply it, show history, then revert and apply again.

5. Optional flush-on-close demo: stage an edit **without** apply, end the MCP
   session gracefully — default flush should commit staged txs. Contrast with
   `--no-flush-on-close` (or `DOTALL_MCP_FLUSH_ON_CLOSE=0`).

## Tips

- Close or refresh Excel after apply so you see the new values.
- `.all/` under `demo/` is gitignored runtime state — safe to delete and regenerate.
- Prefer the **release** binary in MCP config; `cargo run` is slow for live demos.
