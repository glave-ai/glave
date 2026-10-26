#!/bin/sh
# Makes sales.xlsx as a spreadsheet app saves it, and is removed once it has (task.toml,
# setup): a year of sales by month and region, months as dates, each revenue a formula with
# its computed value, and a Summary sheet of full-year totals.
set -eu
parts=$(mktemp -d)
trap 'rm -rf "$parts"' EXIT
mkdir -p "$parts/_rels" "$parts/xl/_rels" "$parts/xl/worksheets"
ns='xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"'
rel=http://schemas.openxmlformats.org/officeDocument/2006/relationships
type=application/vnd.openxmlformats-officedocument.spreadsheetml
head='<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'

echo "$head<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"$type.sheet.main+xml\"/><Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"$type.worksheet+xml\"/><Override PartName=\"/xl/worksheets/sheet2.xml\" ContentType=\"$type.worksheet+xml\"/><Override PartName=\"/xl/styles.xml\" ContentType=\"$type.styles+xml\"/><Override PartName=\"/xl/sharedStrings.xml\" ContentType=\"$type.sharedStrings+xml\"/></Types>" > "$parts/[Content_Types].xml"
echo "$head<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"$rel/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>" > "$parts/_rels/.rels"
echo "$head<workbook $ns><sheets><sheet name=\"Sales\" sheetId=\"1\" r:id=\"rId1\"/><sheet name=\"Summary\" sheetId=\"2\" r:id=\"rId2\"/></sheets><calcPr calcId=\"191029\"/></workbook>" > "$parts/xl/workbook.xml"
echo "$head<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"$rel/worksheet\" Target=\"worksheets/sheet1.xml\"/><Relationship Id=\"rId2\" Type=\"$rel/worksheet\" Target=\"worksheets/sheet2.xml\"/><Relationship Id=\"rId3\" Type=\"$rel/styles\" Target=\"styles.xml\"/><Relationship Id=\"rId4\" Type=\"$rel/sharedStrings\" Target=\"sharedStrings.xml\"/></Relationships>" > "$parts/xl/_rels/workbook.xml.rels"
# Styles: 1 a header, 2 a month (mmm yyyy), 3 money, 4 a share.
echo "$head<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><numFmts count=\"3\"><numFmt numFmtId=\"164\" formatCode=\"mmm yyyy\"/><numFmt numFmtId=\"165\" formatCode=\"\$#,##0.00\"/><numFmt numFmtId=\"166\" formatCode=\"0.0%\"/></numFmts><fonts count=\"2\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font><font><b/><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts><fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill></fills><borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs><cellXfs count=\"5\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/><xf numFmtId=\"0\" fontId=\"1\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyFont=\"1\"/><xf numFmtId=\"164\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/><xf numFmtId=\"165\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/><xf numFmtId=\"166\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/></cellXfs></styleSheet>" > "$parts/xl/styles.xml"
# Shared strings: 0 to 4 the headers, 5 to 7 the regions, 8 and 9 the Summary's headers.
strings=""
for text in Month Region Units Price Revenue North South West "Revenue (full year)" Share; do
  strings="$strings<si><t>$text</t></si>"
done
echo "$head<sst xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" uniqueCount=\"10\">$strings</sst>" > "$parts/xl/sharedStrings.xml"

# Each month's first day as Excel counts days, then units sold in the North, South and West.
rows=$(awk '
  BEGIN {
    printf "<row r=\"1\">"
    for (c = 0; c < 5; c++) printf "<c r=\"%c1\" s=\"1\" t=\"s\"><v>%d</v></c>", 65 + c, c
    printf "</row>"
    split("19.5 21.25 18", price, " ")
    r = 1
  }
  {
    for (g = 1; g <= 3; g++) {
      r++
      printf "<row r=\"%d\"><c r=\"A%d\" s=\"2\"><v>%d</v></c><c r=\"B%d\" t=\"s\"><v>%d</v></c>", r, r, $1, r, 4 + g
      printf "<c r=\"C%d\"><v>%d</v></c><c r=\"D%d\" s=\"3\"><v>%s</v></c>", r, $(g + 1), r, price[g]
      printf "<c r=\"E%d\" s=\"3\"><f>C%d*D%d</f><v>%.10g</v></c></row>", r, r, r, $(g + 1) * price[g]
    }
  }' <<'EOF'
46023 120 100 80
46054 95 105 90
46082 130 98 85
46113 110 120 95
46143 140 115 100
46174 125 130 110
46204 150 125 160
46235 160 118 175
46266 145 140 190
46296 170 150 185
46327 180 135 200
46357 210 160 230
EOF
)
echo "$head<worksheet $ns><sheetViews><sheetView workbookViewId=\"0\"><pane ySplit=\"1\" topLeftCell=\"A2\" activePane=\"bottomLeft\" state=\"frozen\"/></sheetView></sheetViews><sheetData>$rows</sheetData></worksheet>" > "$parts/xl/worksheets/sheet1.xml"

summary="<row r=\"1\"><c r=\"A1\" s=\"1\" t=\"s\"><v>1</v></c><c r=\"B1\" s=\"1\" t=\"s\"><v>8</v></c><c r=\"C1\" s=\"1\" t=\"s\"><v>9</v></c></row>"
r=1
for total in "5 33832.5 0.351606952635818" "6 31790 0.330380108602458" "7 30600 0.318012938761724"; do
  set -- $total
  r=$((r + 1))
  summary="$summary<row r=\"$r\"><c r=\"A$r\" t=\"s\"><v>$1</v></c><c r=\"B$r\" s=\"3\"><f>SUMIF(Sales!\$B\$2:\$B\$37,A$r,Sales!\$E\$2:\$E\$37)</f><v>$2</v></c><c r=\"C$r\" s=\"4\"><f>B$r/SUM(\$B\$2:\$B\$4)</f><v>$3</v></c></row>"
done
echo "$head<worksheet $ns><sheetData>$summary</sheetData></worksheet>" > "$parts/xl/worksheets/sheet2.xml"

rm -f sales.xlsx
(cd "$parts" && zip -q -X -r "$OLDPWD/sales.xlsx" '[Content_Types].xml' _rels xl)
