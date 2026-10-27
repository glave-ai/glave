#!/bin/sh
# Makes memo.docx as Word saves it, and is removed once it has (task.toml, setup): a memo
# about an office move whose paragraph with the dates has its words split across runs, the
# dates bold in a font of its own; a heading, a list, a table and a header around it.
set -eu
parts=$(mktemp -d)
trap 'rm -rf "$parts"' EXIT
mkdir -p "$parts/_rels" "$parts/word/_rels" "$parts/docProps"
head='<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
w='xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"'
ns="xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" $w xmlns:w14=\"http://schemas.microsoft.com/office/word/2010/wordml\" mc:Ignorable=\"w14\""
rel=http://schemas.openxmlformats.org/officeDocument/2006/relationships
type=application/vnd.openxmlformats-officedocument.wordprocessingml

cat > "$parts/[Content_Types].xml" <<EOF
$head
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="$type.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="$type.styles+xml"/><Override PartName="/word/settings.xml" ContentType="$type.settings+xml"/><Override PartName="/word/numbering.xml" ContentType="$type.numbering+xml"/><Override PartName="/word/header1.xml" ContentType="$type.header+xml"/><Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/><Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/></Types>
EOF
cat > "$parts/_rels/.rels" <<EOF
$head
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId3" Type="$rel/extended-properties" Target="docProps/app.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/><Relationship Id="rId1" Type="$rel/officeDocument" Target="word/document.xml"/></Relationships>
EOF
cat > "$parts/word/_rels/document.xml.rels" <<EOF
$head
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="$rel/styles" Target="styles.xml"/><Relationship Id="rId2" Type="$rel/settings" Target="settings.xml"/><Relationship Id="rId3" Type="$rel/header" Target="header1.xml"/><Relationship Id="rId4" Type="$rel/numbering" Target="numbering.xml"/></Relationships>
EOF
cat > "$parts/word/styles.xml" <<EOF
$head
<w:styles $w><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri"/><w:sz w:val="22"/><w:lang w:val="en-GB"/></w:rPr></w:rPrDefault></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style><w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/><w:basedOn w:val="Normal"/><w:qFormat/><w:rPr><w:sz w:val="48"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:outlineLvl w:val="1"/></w:pPr><w:rPr><w:b/><w:sz w:val="28"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="ListBullet"><w:name w:val="List Bullet"/><w:basedOn w:val="Normal"/><w:pPr><w:numPr><w:numId w:val="1"/></w:numPr><w:ind w:left="360" w:hanging="360"/></w:pPr></w:style><w:style w:type="table" w:styleId="TableGrid"><w:name w:val="Table Grid"/></w:style></w:styles>
EOF
cat > "$parts/word/settings.xml" <<EOF
$head
<w:settings $w><w:zoom w:percent="100"/><w:defaultTabStop w:val="720"/><w:characterSpacingControl w:val="doNotCompress"/><w:rsids><w:rsidRoot w:val="00B14C2A"/><w:rsid w:val="00B14C2A"/><w:rsid w:val="00C7D91E"/></w:rsids></w:settings>
EOF
cat > "$parts/word/numbering.xml" <<EOF
$head
<w:numbering $w><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="bullet"/><w:lvlText w:val="&#8226;"/><w:lvlJc w:val="left"/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>
EOF
cat > "$parts/word/header1.xml" <<EOF
$head
<w:hdr $w><w:p><w:r><w:t>Internal memo</w:t></w:r></w:p></w:hdr>
EOF
cat > "$parts/docProps/core.xml" <<EOF
$head
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Office move</dc:title></cp:coreProperties>
EOF
cat > "$parts/docProps/app.xml" <<EOF
$head
<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><Application>Microsoft Office Word</Application></Properties>
EOF

# The dates' paragraph as Word keeps it after edits: words split across runs, the dates bold.
look='<w:rFonts w:ascii="Georgia" w:hAnsi="Georgia"/><w:color w:val="1F3864"/><w:sz w:val="24"/>'
bold='<w:rFonts w:ascii="Georgia" w:hAnsi="Georgia"/><w:b/><w:color w:val="1F3864"/><w:sz w:val="24"/>'
run() { printf '<w:r%s><w:rPr>%s</w:rPr><w:t%s>%s</w:t></w:r>' "$1" "$2" "$3" "$4"; }
keep=' xml:space="preserve"'
edited=' w:rsidR="00C7D91E"'
dates="<w:p w14:paraId=\"4C2A0004\" w:rsidR=\"00B14C2A\" w:rsidRDefault=\"00C7D91E\"><w:pPr><w:spacing w:before=\"120\" w:after=\"120\"/><w:jc w:val=\"both\"/></w:pPr>\
$(run '' "$look" "$keep" 'The move is ')$(run "$edited" "$look" "$keep" 'planned for ')\
$(run '' "$bold" '' 'Fri')$(run "$edited" "$bold" '' 'day, 14 November')\
$(run '' "$look" "$keep" '. Please pack your desks by ')<w:proofErr w:type=\"gramStart\"/>\
$(run '' "$bold" '' 'Thursday, 13 November')<w:proofErr w:type=\"gramEnd\"/>$(run '' "$look" '' '.')</w:p>"
para() { printf '<w:p w:rsidR="00B14C2A" w:rsidRDefault="00B14C2A">%s<w:r><w:t>%s</w:t></w:r></w:p>' "$1" "$2"; }
style() { printf '<w:pPr><w:pStyle w:val="%s"/></w:pPr>' "$1"; }
cell() { printf '<w:tc><w:tcPr><w:tcW w:w="3005" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>%s</w:t></w:r></w:p></w:tc>' "$1"; }
row() { printf '<w:tr w:rsidR="00B14C2A">%s%s%s</w:tr>' "$(cell "$1")" "$(cell "$2")" "$(cell "$3")"; }
body="$(para "$(style Title)" 'Office move')$(para '' 'To: everyone on the third floor')\
$(para '' 'We are moving to the new building on Station Road, two streets from here.')$dates\
$(para "$(style Heading2)" 'What to pack')$(para "$(style ListBullet)" 'Your laptop and its charger')\
$(para "$(style ListBullet)" 'Anything personal on your desk')\
<w:tbl><w:tblPr><w:tblStyle w:val=\"TableGrid\"/><w:tblW w:w=\"0\" w:type=\"auto\"/></w:tblPr><w:tblGrid><w:gridCol w:w=\"3005\"/><w:gridCol w:w=\"3005\"/><w:gridCol w:w=\"3005\"/></w:tblGrid>\
$(row Team 'Old room' 'New room')$(row Sales 3.12 2.14)$(row Support 3.20 2.02)</w:tbl>\
<w:p w:rsidR=\"00B14C2A\" w:rsidRDefault=\"00B14C2A\"><w:r><w:t xml:space=\"preserve\">Questions go to the </w:t></w:r><w:r><w:rPr><w:i/></w:rPr><w:t>facilities team</w:t></w:r><w:r><w:t>.</w:t></w:r></w:p>\
<w:sectPr w:rsidR=\"00B14C2A\"><w:headerReference w:type=\"default\" r:id=\"rId3\"/><w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\" w:header=\"708\" w:footer=\"708\" w:gutter=\"0\"/></w:sectPr>"
printf '%s\r\n<w:document %s><w:body>%s</w:body></w:document>' "$head" "$ns" "$body" > "$parts/word/document.xml"

rm -f memo.docx
(cd "$parts" && zip -q -X -D -r "$OLDPWD/memo.docx" '[Content_Types].xml' _rels word docProps)
