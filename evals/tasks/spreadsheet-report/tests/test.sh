#!/usr/bin/env bash
# report.xlsx is a real workbook: an Orders sheet with all 24 orders and numeric amounts, a
# Summary sheet whose region totals are formulas over Orders, and a column chart of the
# Summary's totals. Read from the file's own XML, so any program may have written it.
set -euo pipefail
fail() { echo "$1"; exit 1; }
[ -f report.xlsx ] || fail "no report.xlsx"
part() { unzip -p report.xlsx "$1" 2>/dev/null || true; }
workbook=$(part xl/workbook.xml)
[ -n "$workbook" ] || fail "report.xlsx is not a workbook (no xl/workbook.xml)"
rels=$(part xl/_rels/workbook.xml.rels)
# The XML of the sheet named $1, found through the workbook's relationships.
sheet() {
  local id target
  id=$(grep -o "<sheet [^>]*name=\"$1\"[^>]*>" <<<"$workbook" | grep -o 'r:id="[^"]*"' | cut -d'"' -f2 || true)
  [ -n "$id" ] || fail "no sheet named $1"
  target=$(grep -o "<Relationship [^>]*Id=\"$id\"[^>]*>" <<<"$rels" | grep -o 'Target="[^"]*"' | cut -d'"' -f2)
  case "$target" in /*) part "${target#/}" ;; *) part "xl/$target" ;; esac
}
orders=$(sheet Orders)
summary=$(sheet Summary)
rows=$(grep -o '<row [^>]*>' <<<"$orders" | wc -l | tr -d ' ')
[ "$rows" -eq 25 ] || fail "Orders has $rows rows, not a header and 24 orders"
# Amounts are the fifth column, E: a number's cell has no type, text's has one (s, str, inlineStr).
amounts=$(grep -oE '<c r="E([2-9]|1[0-9]|2[0-5])"[^>]*>' <<<"$orders" || true)
[ "$(grep -c . <<<"$amounts")" -eq 24 ] || fail "Orders does not hold 24 amounts in column E"
if grep -q ' t="' <<<"$amounts"; then fail "some amounts in Orders are text, not numbers"; fi
formulas=$(grep -o '<f[^>]*>[^<]*</f>' <<<"$summary" | grep -c 'Orders' || true)
[ "$formulas" -ge 3 ] || fail "Summary has $formulas formulas over the Orders sheet, not one per region"
strings="$summary$(part xl/sharedStrings.xml)"
for region in North South West; do
  grep -q ">$region<" <<<"$strings" || fail "Summary does not name $region"
done
charts=$(unzip -Z1 report.xlsx | grep -E '^xl/charts/chart[0-9]*\.xml$' || true)
[ -n "$charts" ] || fail "report.xlsx has no chart"
found=no
for chart in $charts; do
  xml=$(part "$chart")
  if grep -q '<c:barChart>' <<<"$xml" && grep -q '<c:barDir val="col"/>' <<<"$xml" \
      && grep -q 'Summary' <<<"$xml"; then
    found=yes
  fi
done
[ "$found" = yes ] || fail "no column chart of the Summary sheet's totals"
