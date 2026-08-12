# Film prompts — open Cursor on `yc_demo/`

Recording flow:

1. Show `saas_model.xlsx` in Excel (Assumptions + Board) — baseline
2. Paste **Prompt 1** in Cursor — wait until it finishes writing the file
3. Reopen Excel — show the edited sheet
4. Paste **Prompt 2** — wait until it finishes
5. Reopen Excel — show the reverted sheet

Dotall MCP should already be connected via `.cursor/mcp.json`. No skill file needed.

---

## Prompt 1 — edit

```
Look at saas_model.xlsx — our FY2026 SaaS operating model.

We're running a pricing experiment. Update the assumptions: list price $49 → $59, monthly growth 8% → 12%, churn 2% → 1.5%, and COGS 22% → 18%. Also bump January unit volume for every region by 25% (so Americas 1200 → 1500, and the same uplift across the other regions).
```

---

## Prompt 2 — revert

```
Actually, scrap that experiment. Restore saas_model.xlsx to how it was before those edits.
```
