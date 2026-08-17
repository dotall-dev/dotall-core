#!/usr/bin/env bash
# Q3 board-pack bake-off — Dotall CLI walkthrough.
# Copies the committed pack into a scratch workspace so demo/ binaries stay clean.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DOTALL="${DOTALL:-$ROOT/target/debug/dotall}"
WORK="${1:-${TMPDIR:-/tmp}/dotall-q3-demo}"

if [[ ! -x "$DOTALL" ]]; then
  echo "building dotall-cli…"
  cargo build -p dotall-cli --quiet --manifest-path "$ROOT/Cargo.toml"
fi

rm -rf "$WORK"
mkdir -p "$WORK"
cp "$ROOT/demo/q3-pack/q3-financials.xlsx" \
   "$ROOT/demo/q3-pack/q3-deck.pptx" \
   "$ROOT/demo/q3-pack/q3-memo.docx" \
   "$ROOT/demo/q3-pack/q3-intake.pdf" \
   "$WORK/"

XLSX="$WORK/q3-financials.xlsx"
PPTX="$WORK/q3-deck.pptx"
DOCX="$WORK/q3-memo.docx"
PDF="$WORK/q3-intake.pdf"

section() {
  printf '\n\n======== %s ========\n' "$1"
}

section "1. Brief"
cat <<'EOF'
Prepare the Q3 board pack. Commission rate is now 15%.
Update the workbook, then make the deck KPIs, the memo, and the intake form match.
Do not break formulas, the metrics table, or Word styles.
EOF

section "2. Init workspace"
"$DOTALL" init "$WORK"

section "3. Inspect four files (materialize cached models/views)"
for file in "$XLSX" "$PPTX" "$DOCX" "$PDF"; do
  echo "--- inspect $(basename "$file") ---"
  "$DOTALL" inspect "$file" | python3 -c '
import json, sys
d = json.load(sys.stdin)
ins = d.get("inspection") or d
fmt = ins.get("format_id", "?")
summary = ins.get("summary") or {}
print(f"  format={fmt}")
if "named_ranges" in summary:
    print("  named_ranges:", summary["named_ranges"])
if "sheets" in summary:
    print("  sheets:", [s.get("name") for s in summary["sheets"][:4]], "…")
if "slides" in summary:
    slides = summary["slides"]
    print(f"  slides={len(slides)}")
    if slides:
        print("  slide0:", slides[0])
if "fields" in summary:
    print("  fields:", [f.get("name") for f in summary["fields"]])
'
done

section "4. Search cached views (not ZIP XML) — query 10%"
echo "(decoys exist; live targets are Rate / KPI / Q3 line / Rate field)"
"$DOTALL" --json search "10%" "$WORK" | python3 -c '
import json, sys
d = json.load(sys.stdin)
hits = d.get("hits") or []
print("  hits=%s  not_indexed=%s" % (len(hits), d.get("not_indexed")))
for hit in hits[:12]:
    snippet = (hit.get("snippet") or "")[:80]
    print("  %s\t%s\t%s\t%s" % (hit.get("path"), hit.get("selector_kind") or "", hit.get("selector") or "", snippet))
if len(hits) > 12:
    print("  … %s more" % (len(hits) - 12))
'

section "5. Search Rate + read named range + quote commission formula"
"$DOTALL" search "Rate" --glob "q3-financials.xlsx" "$WORK"
echo
"$DOTALL" read "$XLSX" --selector-kind named_ranges
echo
"$DOTALL" read "$XLSX" --range "Inputs!B2"
echo
"$DOTALL" deps "$XLSX" --cell "Revenue!B5" || true

section "6. Surgical edits — xlsx → pptx → docx → pdf"
echo "xlsx: Inputs!B2 0.10 → 0.15 (named range Rate; formula Revenue!B5 stays)"
"$DOTALL" edit "$XLSX" --ops-json \
  '[{"kind":"set_cell_value","payload":{"sheet":"Inputs","address":"B2","value":0.15}}]'
"$DOTALL" apply "$XLSX" --all

echo
echo "pptx: Slide 1 shape KPI 10% → 15%"
"$DOTALL" edit "$PPTX" --ops-json \
  '[{"kind":"replace_shape_text","payload":{"slide":"Slide 1","shape":"KPI","find":"10%","replace":"15%"}}]'
"$DOTALL" apply "$PPTX" --all

echo
echo "docx: live Q3 line only (decoy 10% paragraphs stay)"
"$DOTALL" edit "$DOCX" --ops-json \
  '[{"kind":"replace_across_paragraphs","payload":{"find":"Q3 commission rate: 10%.","replace":"Q3 commission rate: 15%."}}]'
"$DOTALL" apply "$DOCX" --all

echo
echo "pdf: AcroForm Rate 10% → 15%"
"$DOTALL" edit "$PDF" --ops-json \
  '[{"kind":"set_form_field","payload":{"name":"Rate","value":"15%"}}]'
"$DOTALL" apply "$PDF" --all

section "7. Confirm live 15% (search should now hit the four targets)"
"$DOTALL" --json search "15%" "$WORK" | python3 -c '
import json, sys
d = json.load(sys.stdin)
hits = d.get("hits") or []
print("  hits=%s" % len(hits))
for hit in hits:
    snippet = (hit.get("snippet") or "")[:90]
    print("  %s\t%s\t%s\t%s" % (hit.get("path"), hit.get("selector_kind") or "", hit.get("selector") or "", snippet))
'

section "8. History (xlsx)"
"$DOTALL" history "$XLSX"

section "9. Workspace"
echo "WORK=$WORK"
echo "Next: $DOTALL viz --port 8765 \"$WORK\""
echo "$WORK" > "$WORK/.demo-workspace"
