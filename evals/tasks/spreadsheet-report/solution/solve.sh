#!/usr/bin/env bash
# The workbook written by hand as its XML parts, zipped: the reference needs nothing but zip.
set -euo pipefail
parts=$(mktemp -d)
trap 'rm -rf "$parts"' EXIT
mkdir -p "$parts/_rels" "$parts/xl/_rels" "$parts/xl/worksheets/_rels" "$parts/xl/drawings/_rels" "$parts/xl/charts"
main='xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"'
rel=http://schemas.openxmlformats.org/officeDocument/2006/relationships
type=application/vnd.openxmlformats-officedocument
text() { printf '<c r="%s" t="inlineStr"><is><t>%s</t></is></c>' "$1" "$2"; }

cat > "$parts/[Content_Types].xml" <<EOF
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="$type.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="$type.spreadsheetml.worksheet+xml"/><Override PartName="/xl/worksheets/sheet2.xml" ContentType="$type.spreadsheetml.worksheet+xml"/><Override PartName="/xl/drawings/drawing1.xml" ContentType="$type.drawing+xml"/><Override PartName="/xl/charts/chart1.xml" ContentType="$type.drawingml.chart+xml"/></Types>
EOF
cat > "$parts/_rels/.rels" <<EOF
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="$rel/officeDocument" Target="xl/workbook.xml"/></Relationships>
EOF
cat > "$parts/xl/workbook.xml" <<EOF
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook $main><sheets><sheet name="Orders" sheetId="1" r:id="rId1"/><sheet name="Summary" sheetId="2" r:id="rId2"/></sheets><calcPr fullCalcOnLoad="1"/></workbook>
EOF
cat > "$parts/xl/_rels/workbook.xml.rels" <<EOF
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="$rel/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="$rel/worksheet" Target="worksheets/sheet2.xml"/></Relationships>
EOF

IFS=, read -r -a heads < orders.csv
rows="<row r=\"1\">$(text A1 "${heads[0]}")$(text B1 "${heads[1]}")$(text C1 "${heads[2]}")"
rows+="$(text D1 "${heads[3]}")$(text E1 "${heads[4]}")</row>"
n=1
while IFS=, read -r order date region customer amount; do
  n=$((n + 1))
  rows+="<row r=\"$n\"><c r=\"A$n\"><v>$order</v></c>$(text "B$n" "$date")$(text "C$n" "$region")$(text "D$n" "$customer")<c r=\"E$n\"><v>$amount</v></c></row>"
done < <(tail -n +2 orders.csv)
printf '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<worksheet %s><sheetData>%s</sheetData></worksheet>\n' "$main" "$rows" > "$parts/xl/worksheets/sheet1.xml"

summary="<row r=\"1\">$(text A1 Region)$(text B1 Total)</row>"
r=1
for region in North South West; do
  r=$((r + 1))
  summary+="<row r=\"$r\">$(text "A$r" "$region")<c r=\"B$r\"><f>SUMIF(Orders!\$C\$2:\$C\$$n,A$r,Orders!\$E\$2:\$E\$$n)</f></c></row>"
done
printf '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<worksheet %s><sheetData>%s</sheetData><drawing r:id="rId1"/></worksheet>\n' "$main" "$summary" > "$parts/xl/worksheets/sheet2.xml"
cat > "$parts/xl/worksheets/_rels/sheet2.xml.rels" <<EOF
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="$rel/drawing" Target="../drawings/drawing1.xml"/></Relationships>
EOF
cat > "$parts/xl/drawings/drawing1.xml" <<EOF
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><xdr:twoCellAnchor><xdr:from><xdr:col>3</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>10</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>16</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to><xdr:graphicFrame macro=""><xdr:nvGraphicFramePr><xdr:cNvPr id="2" name="Chart 1"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/></xdr:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:r="$rel" r:id="rId1"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData/></xdr:twoCellAnchor></xdr:wsDr>
EOF
cat > "$parts/xl/drawings/_rels/drawing1.xml.rels" <<EOF
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="$rel/chart" Target="../charts/chart1.xml"/></Relationships>
EOF
cat > "$parts/xl/charts/chart1.xml" <<'EOF'
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><c:chart><c:plotArea><c:layout/><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/><c:ser><c:idx val="0"/><c:order val="0"/><c:cat><c:strRef><c:f>Summary!$A$2:$A$4</c:f></c:strRef></c:cat><c:val><c:numRef><c:f>Summary!$B$2:$B$4</c:f></c:numRef></c:val></c:ser><c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:axPos val="b"/><c:crossAx val="2"/></c:catAx><c:valAx><c:axId val="2"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:axPos val="l"/><c:crossAx val="1"/></c:valAx></c:plotArea></c:chart></c:chartSpace>
EOF
rm -f report.xlsx
(cd "$parts" && zip -q -X -r "$OLDPWD/report.xlsx" '[Content_Types].xml' _rels xl)
