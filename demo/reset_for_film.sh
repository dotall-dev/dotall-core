#!/usr/bin/env bash
# Reset demo/ for a clean film take and refresh .cursor/mcp.json paths.
set -euo pipefail
DEMO="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$DEMO/.." && pwd)"
DOTALL="${DOTALL:-$ROOT/target/release/dotall}"
MCP="${MCP:-$ROOT/target/release/dotall-mcp}"

if [[ ! -x "$MCP" ]]; then
  echo "missing $MCP — run: cargo build --release -p dotall-mcp" >&2
  exit 1
fi

# Pristine workbook
if command -v cargo >/dev/null 2>&1; then
  (cd "$ROOT" && cargo run -q -p dotall-xlsx --example gen_yc_demo_workbook)
fi

rm -rf "$DEMO/.all"
"$DOTALL" init "$DEMO" >/dev/null
"$DOTALL" inspect "$DEMO/saas_model.xlsx" >/dev/null

mkdir -p "$DEMO/.cursor" "$DEMO/skills/xlsx"
if [[ -f "$ROOT/skills/xlsx/SKILL.md" ]]; then
  cp "$ROOT/skills/xlsx/SKILL.md" "$DEMO/skills/xlsx/SKILL.md"
fi

python3 - "$MCP" "$DEMO" <<'PY'
import json, sys
from pathlib import Path
mcp, demo = sys.argv[1], sys.argv[2]
cfg = {
    "mcpServers": {
        "dotall": {
            "command": mcp,
            "args": [],
            "cwd": demo,
        }
    }
}
path = Path(demo) / ".cursor" / "mcp.json"
path.write_text(json.dumps(cfg, indent=2) + "\n")
print(f"wrote {path}")
print(path.read_text())
PY

echo "Ready. Open Cursor on: $DEMO"
echo "Prompts: $DEMO/PROMPTS.md"
