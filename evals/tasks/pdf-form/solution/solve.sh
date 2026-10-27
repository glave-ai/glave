#!/usr/bin/env bash
# The fields' values added as an update at the end of the file, as a PDF editor saves one, and
# the form told to have viewers draw them. Python's standard library alone.
set -euo pipefail
python3 -I - <<'EOF'
import re

data = open("leave-request.pdf", "rb").read()
changes = {
    4: [(b" >> >>", b" >> /NeedAppearances true >>")],
    10: [(b" >>", b" /V (Sam Okafor) >>")],
    11: [(b" >>", b" /V (Support) >>")],
    12: [(b" >>", b" /V (07/12/2026) >>")],
    13: [(b" >>", b" /V (11/12/2026) >>")],
    14: [(b" >>", b" /V (5) >>")],
    15: [(b"/V /Off", b"/V /Annual")],
    16: [(b"/AS /Off", b"/AS /Annual")],
    19: [(b"/V /Off /AS /Off", b"/V /Yes /AS /Yes")],
}
out = bytearray(data)
offsets = {}
for number, edits in sorted(changes.items()):
    found = re.search(rb"\n%d 0 obj\n(.*?)\nendobj" % number, data, re.S).group(1)
    for old, new in edits:
        at = found.rindex(old)
        found = found[:at] + new + found[at + len(old):]
    offsets[number] = len(out)
    out += b"%d 0 obj\n%s\nendobj\n" % (number, found)
xref = len(out)
out += b"xref\n"
for number, offset in sorted(offsets.items()):
    out += b"%d 1\n%010d 00000 n \n" % (number, offset)
previous = int(re.findall(rb"startxref\s+(\d+)", data)[-1])
out += b"trailer\n<< /Size 22 /Root 1 0 R /Prev %d >>\nstartxref\n%d\n%%%%EOF\n" % (previous, xref)
open("leave-request-filled.pdf", "wb").write(out)
EOF
