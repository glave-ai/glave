#!/bin/sh
# Makes budget.xlsx as Excel saves it, and is removed once it has (task.toml, setup): a year's
# budget by quarter with a row total for each item and a column total for each quarter, all
# formulas with the values they last computed to. Travel's row total leaves out Q4. Around it:
# money formats, a bold shaded header, column widths, a frozen header row, a highlight on
# items over $100,000, a defined name, a column chart and a second sheet.
set -eu
parts=$(mktemp -d)
trap 'rm -rf "$parts"' EXIT
mkdir -p "$parts/_rels" "$parts/docProps" "$parts/xl/_rels" "$parts/xl/worksheets/_rels" \
  "$parts/xl/drawings/_rels" "$parts/xl/charts"
head='<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
main='xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"'
rel=http://schemas.openxmlformats.org/officeDocument/2006/relationships
rels='xmlns="http://schemas.openxmlformats.org/package/2006/relationships"'
type=application/vnd.openxmlformats-officedocument
part() { printf '%s\r\n%s' "$head" "$2" > "$parts/$1"; }

part '[Content_Types].xml' "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"$type.spreadsheetml.sheet.main+xml\"/><Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"$type.spreadsheetml.worksheet+xml\"/><Override PartName=\"/xl/worksheets/sheet2.xml\" ContentType=\"$type.spreadsheetml.worksheet+xml\"/><Override PartName=\"/xl/styles.xml\" ContentType=\"$type.spreadsheetml.styles+xml\"/><Override PartName=\"/xl/sharedStrings.xml\" ContentType=\"$type.spreadsheetml.sharedStrings+xml\"/><Override PartName=\"/xl/drawings/drawing1.xml\" ContentType=\"$type.drawing+xml\"/><Override PartName=\"/xl/charts/chart1.xml\" ContentType=\"$type.drawingml.chart+xml\"/><Override PartName=\"/docProps/app.xml\" ContentType=\"$type.extended-properties+xml\"/></Types>"
part _rels/.rels "<Relationships $rels><Relationship Id=\"rId2\" Type=\"$rel/extended-properties\" Target=\"docProps/app.xml\"/><Relationship Id=\"rId1\" Type=\"$rel/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>"
part docProps/app.xml "<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>Microsoft Excel</Application></Properties>"
part xl/workbook.xml "<workbook $main><bookViews><workbookView xWindow=\"0\" yWindow=\"0\" windowWidth=\"28800\" windowHeight=\"12300\"/></bookViews><sheets><sheet name=\"Budget\" sheetId=\"1\" r:id=\"rId1\"/><sheet name=\"Notes\" sheetId=\"2\" r:id=\"rId2\"/></sheets><definedNames><definedName name=\"Year\">Budget!\$F\$2:\$F\$6</definedName></definedNames><calcPr calcId=\"191029\"/></workbook>"
part xl/_rels/workbook.xml.rels "<Relationships $rels><Relationship Id=\"rId1\" Type=\"$rel/worksheet\" Target=\"worksheets/sheet1.xml\"/><Relationship Id=\"rId2\" Type=\"$rel/worksheet\" Target=\"worksheets/sheet2.xml\"/><Relationship Id=\"rId3\" Type=\"$rel/styles\" Target=\"styles.xml\"/><Relationship Id=\"rId4\" Type=\"$rel/sharedStrings\" Target=\"sharedStrings.xml\"/></Relationships>"
# Styles: 1 the header (bold, shaded), 2 money, 3 money in bold for the totals; a red font
# for the highlight.
part xl/styles.xml "<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><numFmts count=\"1\"><numFmt numFmtId=\"164\" formatCode=\"&quot;\$&quot;#,##0\"/></numFmts><fonts count=\"2\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font><font><b/><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts><fills count=\"3\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill><fill><patternFill patternType=\"solid\"><fgColor rgb=\"FFD9E1F2\"/><bgColor indexed=\"64\"/></patternFill></fill></fills><borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs><cellXfs count=\"4\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/><xf numFmtId=\"0\" fontId=\"1\" fillId=\"2\" borderId=\"0\" xfId=\"0\" applyFont=\"1\" applyFill=\"1\"/><xf numFmtId=\"164\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/><xf numFmtId=\"164\" fontId=\"1\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\" applyFont=\"1\"/></cellXfs><dxfs count=\"1\"><dxf><font><color rgb=\"FF9C0006\"/></font></dxf></dxfs></styleSheet>"
# Shared strings: 0 to 5 the header, 6 to 10 the items, 11 Total, 12 the note.
strings=""
for text in Item Q1 Q2 Q3 Q4 Year Rent Salaries Travel Software Training Total \
  "Figures are in dollars, before tax."; do
  strings="$strings<si><t>$text</t></si>"
done
part xl/sharedStrings.xml "<sst xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" count=\"13\" uniqueCount=\"13\">$strings</sst>"

# Each item's quarters; its row total a formula, Travel's leaving out Q4 (E), as saved.
rows=$(awk '
  BEGIN {
    printf "<row r=\"1\" spans=\"1:6\">"
    for (c = 0; c < 6; c++) printf "<c r=\"%c1\" s=\"1\" t=\"s\"><v>%d</v></c>", 65 + c, c
    printf "</row>"
  }
  {
    r = NR + 1
    printf "<row r=\"%d\" spans=\"1:6\"><c r=\"A%d\" t=\"s\"><v>%d</v></c>", r, r, NR + 5
    for (c = 2; c <= 5; c++) { printf "<c r=\"%c%d\" s=\"2\"><v>%d</v></c>", 64 + c, r, $c; total[c] += $c }
    last = (r == 4) ? "D" : "E"
    year = $2 + $3 + $4 + (r == 4 ? 0 : $5)
    printf "<c r=\"F%d\" s=\"2\"><f>SUM(B%d:%s%d)</f><v>%d</v></c></row>", r, r, last, r, year
    years += year
  }
  END {
    printf "<row r=\"7\" spans=\"1:6\"><c r=\"A7\" s=\"1\" t=\"s\"><v>11</v></c>"
    for (c = 2; c <= 5; c++) printf "<c r=\"%c7\" s=\"3\"><f>SUM(%c2:%c6)</f><v>%d</v></c>", 64 + c, 64 + c, 64 + c, total[c]
    printf "<c r=\"F7\" s=\"3\"><f>SUM(F2:F6)</f><v>%d</v></c></row>", years
  }' <<'EOF'
Rent 12000 12000 12000 12000
Salaries 85000 85000 87000 87000
Travel 4200 3100 5600 6900
Software 2400 2400 2600 2600
Training 1500 0 2500 1000
EOF
)
part xl/worksheets/sheet1.xml "<worksheet $main><dimension ref=\"A1:F7\"/><sheetViews><sheetView tabSelected=\"1\" workbookViewId=\"0\"><pane ySplit=\"1\" topLeftCell=\"A2\" activePane=\"bottomLeft\" state=\"frozen\"/><selection pane=\"bottomLeft\" activeCell=\"A2\" sqref=\"A2\"/></sheetView></sheetViews><sheetFormatPr defaultRowHeight=\"15\"/><cols><col min=\"1\" max=\"1\" width=\"14\" customWidth=\"1\"/><col min=\"2\" max=\"6\" width=\"12\" customWidth=\"1\"/></cols><sheetData>$rows</sheetData><conditionalFormatting sqref=\"F2:F6\"><cfRule type=\"cellIs\" dxfId=\"0\" priority=\"1\" operator=\"greaterThan\"><formula>100000</formula></cfRule></conditionalFormatting><pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/><drawing r:id=\"rId1\"/></worksheet>"
part xl/worksheets/_rels/sheet1.xml.rels "<Relationships $rels><Relationship Id=\"rId1\" Type=\"$rel/drawing\" Target=\"../drawings/drawing1.xml\"/></Relationships>"
part xl/worksheets/sheet2.xml "<worksheet $main><dimension ref=\"A1\"/><sheetViews><sheetView workbookViewId=\"0\"/></sheetViews><sheetFormatPr defaultRowHeight=\"15\"/><sheetData><row r=\"1\"><c r=\"A1\" t=\"s\"><v>12</v></c></row></sheetData><pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/></worksheet>"
part xl/drawings/drawing1.xml "<xdr:wsDr xmlns:xdr=\"http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><xdr:twoCellAnchor><xdr:from><xdr:col>7</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>14</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>16</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to><xdr:graphicFrame macro=\"\"><xdr:nvGraphicFramePr><xdr:cNvPr id=\"2\" name=\"Chart 1\"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/></xdr:xfrm><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/chart\"><c:chart xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\" xmlns:r=\"$rel\" r:id=\"rId1\"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData/></xdr:twoCellAnchor></xdr:wsDr>"
part xl/drawings/_rels/drawing1.xml.rels "<Relationships $rels><Relationship Id=\"rId1\" Type=\"$rel/chart\" Target=\"../charts/chart1.xml\"/></Relationships>"
points=""
i=0
for year in 48000 344000 12900 10000 5000; do
  points="$points<c:pt idx=\"$i\"><c:v>$year</c:v></c:pt>"
  i=$((i + 1))
done
part xl/charts/chart1.xml "<c:chartSpace xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><c:chart><c:title><c:tx><c:rich><a:bodyPr/><a:p><a:r><a:t>Year by item</a:t></a:r></a:p></c:rich></c:tx><c:overlay val=\"0\"/></c:title><c:plotArea><c:barChart><c:barDir val=\"col\"/><c:grouping val=\"clustered\"/><c:ser><c:idx val=\"0\"/><c:order val=\"0\"/><c:cat><c:strRef><c:f>Budget!\$A\$2:\$A\$6</c:f></c:strRef></c:cat><c:val><c:numRef><c:f>Budget!\$F\$2:\$F\$6</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val=\"5\"/>$points</c:numCache></c:numRef></c:val></c:ser><c:axId val=\"1\"/><c:axId val=\"2\"/></c:barChart><c:catAx><c:axId val=\"1\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"b\"/><c:crossAx val=\"2\"/></c:catAx><c:valAx><c:axId val=\"2\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"l\"/><c:crossAx val=\"1\"/></c:valAx></c:plotArea></c:chart></c:chartSpace>"

rm -f budget.xlsx
(cd "$parts" && zip -q -X -D -r "$OLDPWD/budget.xlsx" '[Content_Types].xml' _rels docProps xl)
