#!/usr/bin/env bash
# Rehearse the YC demo apply/revert loop and write proof snapshots.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DOTALL="${DOTALL:-$ROOT/target/release/dotall}"
TAPE="$ROOT/demo/yc-tape"
FILE="$ROOT/demo/saas_model.xlsx"

mkdir -p "$TAPE"
rm -rf "$ROOT/demo/.all"
# Restore pristine workbook from generator if available
if command -v cargo >/dev/null 2>&1; then
  (cd "$ROOT" && cargo run -q -p dotall-xlsx --example gen_yc_demo_workbook)
fi

"$DOTALL" init "$ROOT/demo" >/dev/null
"$DOTALL" inspect "$FILE" >/dev/null

snap() {
  local name="$1"
  {
    echo "=== $name Assumptions!A1:B5 ==="
    "$DOTALL" read "$FILE" --range 'Assumptions!A1:B5'
    echo
    echo "=== $name Regions!B2:B9 ==="
    "$DOTALL" read "$FILE" --range 'Regions!B2:B9'
    echo
    echo "=== $name Board!A1:B7 ==="
    "$DOTALL" read "$FILE" --range 'Board!A1:B7'
  } >"$TAPE/${name}.txt"
  cp -f "$FILE" "$TAPE/${name}.xlsx"
  echo "wrote $TAPE/${name}.txt and $TAPE/${name}.xlsx"
}

snap baseline

OPS='[
  {"kind":"set_range","payload":{"sheet":"Assumptions","start_cell":"B2","values":[[59],[0.12],[0.015],[0.18]]}},
  {"kind":"set_range","payload":{"sheet":"Regions","start_cell":"B2","values":[[1500],[1200],[1000],[500],[400],[700],[800],[350]]}}
]'

{
  echo "=== stage ==="
  "$DOTALL" edit "$FILE" --ops-json "$OPS"
  echo
  echo "=== apply ==="
  "$DOTALL" apply "$FILE" --all
} | tee "$TAPE/apply.log"

snap changed

{
  echo "=== history ==="
  "$DOTALL" history "$FILE"
  echo
  echo "=== diff v1 ==="
  "$DOTALL" diff "$FILE" --version 1
  echo
  echo "=== revert v1 ==="
  "$DOTALL" revert "$FILE" --version 1
  echo
  echo "=== apply revert ==="
  "$DOTALL" apply "$FILE" --all
} | tee "$TAPE/revert.log"

snap reverted

# Leave workspace warm + pristine file for filming
cp -f "$TAPE/baseline.xlsx" "$FILE"
rm -rf "$ROOT/demo/.all"
"$DOTALL" init "$ROOT/demo" >/dev/null
"$DOTALL" inspect "$FILE" >/dev/null

echo "Rehearsal complete. Film from demo/YC_VIDEO.md; cutaways in demo/yc-tape/."
