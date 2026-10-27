#!/usr/bin/env bash
# The words changed inside their own runs, so each keeps its formatting; every other part is
# zipped back as it was. Python's standard library alone.
set -euo pipefail
python3 -I - <<'EOF'
import zipfile

changes = [
    ('<w:t xml:space="preserve">planned for </w:t>', '<w:t xml:space="preserve">now on </w:t>'),
    ("<w:t>Fri</w:t>", "<w:t>Mon</w:t>"),
    ("<w:t>day, 14 November</w:t>", "<w:t>day, 24 November</w:t>"),
    ("<w:t>Thursday, 13 November</w:t>", "<w:t>Friday, 21 November</w:t>"),
]
with zipfile.ZipFile("memo.docx") as book:
    parts = [(info, book.read(info)) for info in book.infolist()]
with zipfile.ZipFile("memo.docx", "w") as book:
    for info, data in parts:
        if info.filename == "word/document.xml":
            text = data.decode("utf-8")
            for old, new in changes:
                assert text.count(old) == 1, old
                text = text.replace(old, new)
            data = text.encode("utf-8")
        book.writestr(info, data)
EOF
