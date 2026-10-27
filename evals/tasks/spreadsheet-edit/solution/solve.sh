#!/usr/bin/env bash
# The formula fixed in its own cell, its old result taken out, and the workbook told to work
# every formula out again when an app opens it; every other part is zipped back as it was.
# Python's standard library alone.
set -euo pipefail
python3 -I - <<'EOF'
import zipfile

changes = {
    "xl/worksheets/sheet1.xml": [("<f>SUM(B4:D4)</f><v>12900</v>", "<f>SUM(B4:E4)</f>")],
    "xl/workbook.xml": [('<calcPr calcId="191029"/>', '<calcPr calcId="191029" fullCalcOnLoad="1"/>')],
}
with zipfile.ZipFile("budget.xlsx") as book:
    parts = [(info, book.read(info)) for info in book.infolist()]
with zipfile.ZipFile("budget.xlsx", "w") as book:
    for info, data in parts:
        text = data.decode("utf-8") if info.filename in changes else None
        for old, new in changes.get(info.filename, []):
            assert text.count(old) == 1, old
            text = text.replace(old, new)
        book.writestr(info, data if text is None else text.encode("utf-8"))
EOF
