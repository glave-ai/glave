#!/bin/sh
# Makes quarters.xlsx as a spreadsheet app saves it, and is removed once it has (task.toml,
# setup): a sheet a quarter of weekly units sold, 117 rows each, and a hidden sheet of planned
# units. Copper Pan leads the first quarter and Brass Lamp the plan; Teal Kettle sold the most.
set -eu
parts=$(mktemp -d)
trap 'rm -rf "$parts"' EXIT
mkdir -p "$parts/_rels" "$parts/xl/_rels" "$parts/xl/worksheets"
ns='xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"'
rel=http://schemas.openxmlformats.org/officeDocument/2006/relationships
type=application/vnd.openxmlformats-officedocument.spreadsheetml
head='<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
products="Copper Pan|Teal Kettle|Brass Lamp|Oak Board|Linen Towel|Clay Mug|Steel Whisk|Glass Jar|Wool Throw"

overrides=""
sheets=""
rels=""
for n in 1 2 3 4 5; do
  overrides="$overrides<Override PartName=\"/xl/worksheets/sheet$n.xml\" ContentType=\"$type.worksheet+xml\"/>"
  rels="$rels<Relationship Id=\"rId$n\" Type=\"$rel/worksheet\" Target=\"worksheets/sheet$n.xml\"/>"
done
for n in 1 2 3 4; do
  sheets="$sheets<sheet name=\"Q$n\" sheetId=\"$n\" r:id=\"rId$n\"/>"
done
sheets="$sheets<sheet name=\"Plan\" sheetId=\"5\" state=\"hidden\" r:id=\"rId5\"/>"
echo "$head<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"$type.sheet.main+xml\"/>$overrides<Override PartName=\"/xl/sharedStrings.xml\" ContentType=\"$type.sharedStrings+xml\"/></Types>" > "$parts/[Content_Types].xml"
echo "$head<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"$rel/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>" > "$parts/_rels/.rels"
echo "$head<workbook $ns><sheets>$sheets</sheets><calcPr calcId=\"191029\"/></workbook>" > "$parts/xl/workbook.xml"
echo "$head<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">$rels<Relationship Id=\"rId6\" Type=\"$rel/sharedStrings\" Target=\"sharedStrings.xml\"/></Relationships>" > "$parts/xl/_rels/workbook.xml.rels"
# Shared strings: 0 to 8 the products, then Week, Product, Units and Planned units.
strings=$(echo "$products|Week|Product|Units|Planned units" | tr '|' '\n' | sed 's|.*|<si><t>&</t></si>|' | tr -d '\n')
echo "$head<sst xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" uniqueCount=\"13\">$strings</sst>" > "$parts/xl/sharedStrings.xml"

# Each product's weekly base in each quarter, then a small weekly swing.
for q in 1 2 3 4; do
  rows=$(awk -v q="$q" '
    BEGIN {
      split("60 45 40 42", copper, " "); split("40 50 58 55", teal, " ")
      split("30 33 35 31", brass, " ")
      split("25 28 30 27 22 26 24 29 31 27 23 25 28 30 26 24 21 23 25 27 26 29 24 22", rest, " ")
      printf "<row r=\"1\"><c r=\"A1\" t=\"s\"><v>9</v></c><c r=\"B1\" t=\"s\"><v>10</v></c><c r=\"C1\" t=\"s\"><v>11</v></c></row>"
      r = 1
      for (week = 1; week <= 13; week++) {
        for (p = 0; p < 9; p++) {
          if (p == 0) base = copper[q]; else if (p == 1) base = teal[q]; else if (p == 2) base = brass[q]
          else base = rest[(p - 3) * 4 + q]
          r++
          printf "<row r=\"%d\"><c r=\"A%d\"><v>%d</v></c><c r=\"B%d\" t=\"s\"><v>%d</v></c>", r, r, (q - 1) * 13 + week, r, p
          printf "<c r=\"C%d\"><v>%d</v></c></row>", r, base + (week + p) % 3
        }
      }
    }')
  echo "$head<worksheet $ns><sheetData>$rows</sheetData></worksheet>" > "$parts/xl/worksheets/sheet$q.xml"
done
plan="<row r=\"1\"><c r=\"A1\" t=\"s\"><v>10</v></c><c r=\"B1\" t=\"s\"><v>12</v></c></row>"
r=1
for planned in "0 2600" "1 2700" "2 4000" "3 1500" "4 1400" "5 1600" "6 1300" "7 1550" "8 1450"; do
  set -- $planned
  r=$((r + 1))
  plan="$plan<row r=\"$r\"><c r=\"A$r\" t=\"s\"><v>$1</v></c><c r=\"B$r\"><v>$2</v></c></row>"
done
echo "$head<worksheet $ns><sheetData>$plan</sheetData></worksheet>" > "$parts/xl/worksheets/sheet5.xml"

rm -f quarters.xlsx
(cd "$parts" && zip -q -X -r "$OLDPWD/quarters.xlsx" '[Content_Types].xml' _rels xl)
