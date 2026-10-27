#!/bin/sh
# Makes leave-request.pdf, and is removed once it has (task.toml, setup): a one-page form for
# time off, as a form-making app saves one. Its fields are named Text1 to Text6, Group1 and
# Check Box1 and 2, not as their labels on the page; each one's description (TU) says what its
# label says. Above the line, the person's part; below it, the manager's.
set -eu
body=$(mktemp)
trap 'rm -f "$body"' EXIT
printf '%%PDF-1.7\n' > "$body"
offsets=""
# Object $1, holding $2; written in number order, so each offset is the next in the list.
obj() {
  offsets="$offsets $(wc -c < "$body" | tr -d ' ')"
  printf '%s 0 obj\n%s\nendobj\n' "$1" "$2" >> "$body"
}
# Object $1, a stream with the entries $2 and the content $3.
stream() {
  obj "$1" "<< $2 /Length $(printf '%s' "$3" | wc -c | tr -d ' ') >>
stream
$3
endstream"
}
# A label at x $1, y $2, size $3.
label() { printf 'BT /F1 %s Tf %s %s Td (%s) Tj ET\n' "$3" "$1" "$2" "$4"; }
# A widget's own entries: its box from x $1, y $2, $3 wide and $4 high, on the page.
box() { printf '/Type /Annot /Subtype /Widget /Rect [%s %s %s %s] /P 3 0 R /F 4' \
  "$1" "$2" "$(($1 + $3))" "$(($2 + $4))"; }
# A text field named $1 and described as $2, at x $3, y $4, $5 wide, with more entries $6.
text() { printf '<< %s /FT /Tx /T (%s) /TU (%s) /DA (/Helv 0 Tf 0 g)%s >>' \
  "$(box "$3" "$4" "$5" 20)" "$1" "$2" "$6"; }
# A checkbox or radio button's looks, on as $1.
looks() { printf '/AP << /N << /%s %s 0 R /Off 8 0 R >> >>' "$1" "$2"; }

content="$(label 60 780 20 'Leave request')
$(label 60 740 13 'Your details')
$(label 60 700 11 'Employee name')
$(label 60 670 11 'Team')
$(label 60 640 11 'First day off (DD/MM/YYYY)')
$(label 60 610 11 'Last day off (DD/MM/YYYY)')
$(label 60 580 11 'Working days')
$(label 60 550 11 'Type of leave')
$(label 268 550 11 'Annual')
$(label 358 550 11 'Sick')
$(label 438 550 11 'Unpaid')
$(label 60 520 11 'My work is handed over')
0.5 w 40 490 m 555 490 l S
$(label 60 460 13 'For your manager')
$(label 60 430 11 'Name')
$(label 60 400 11 'Approved')
250 695 285 20 re 250 665 285 20 re 250 635 150 20 re 250 605 150 20 re 250 575 50 20 re S
250 545 14 14 re 340 545 14 14 re 420 545 14 14 re 250 515 14 14 re S
250 425 285 20 re 250 395 14 14 re S"

obj 1 '<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>'
obj 2 '<< /Type /Pages /Kids [3 0 R] /Count 1 >>'
obj 3 '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 5 0 R >> >> /Contents 6 0 R /Annots [10 0 R 11 0 R 12 0 R 13 0 R 14 0 R 16 0 R 17 0 R 18 0 R 19 0 R 20 0 R 21 0 R] >>'
obj 4 '<< /Fields [10 0 R 11 0 R 12 0 R 13 0 R 14 0 R 15 0 R 19 0 R 20 0 R 21 0 R] /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv 5 0 R >> >> >>'
obj 5 '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>'
stream 6 '' "$content"
look='/Type /XObject /Subtype /Form /BBox [0 0 14 14]'
stream 7 "$look" '1 w 3 3 m 11 11 l 3 11 m 11 3 l S'
stream 8 "$look" ''
stream 9 "$look" '0 g 4 4 6 6 re f'
obj 10 "$(text Text1 'Employee name' 250 695 285 '')"
obj 11 "$(text Text2 'Team' 250 665 285 '')"
obj 12 "$(text Text3 'First day off (DD/MM/YYYY)' 250 635 150 ' /MaxLen 10')"
obj 13 "$(text Text4 'Last day off (DD/MM/YYYY)' 250 605 150 ' /MaxLen 10')"
obj 14 "$(text Text5 'Working days' 250 575 50 ' /MaxLen 2')"
obj 15 '<< /FT /Btn /Ff 49152 /T (Group1) /TU (Type of leave) /V /Off /Kids [16 0 R 17 0 R 18 0 R] >>'
obj 16 "<< $(box 250 545 14 14) /Parent 15 0 R /AS /Off $(looks Annual 9) >>"
obj 17 "<< $(box 340 545 14 14) /Parent 15 0 R /AS /Off $(looks Sick 9) >>"
obj 18 "<< $(box 420 545 14 14) /Parent 15 0 R /AS /Off $(looks Unpaid 9) >>"
obj 19 "<< $(box 250 515 14 14) /FT /Btn /T (Check Box1) /TU (My work is handed over) /V /Off /AS /Off $(looks Yes 7) >>"
obj 20 "$(text Text6 "Manager's name" 250 425 285 '')"
obj 21 "<< $(box 250 395 14 14) /FT /Btn /T (Check Box2) /TU (Approved by the manager) /V /Off /AS /Off $(looks Yes 7) >>"

xref=$(wc -c < "$body" | tr -d ' ')
{
  printf 'xref\n0 22\n0000000000 65535 f \n'
  for offset in $offsets; do
    printf '%010d 00000 n \n' "$offset"
  done
  printf 'trailer\n<< /Size 22 /Root 1 0 R >>\nstartxref\n%s\n%%%%EOF\n' "$xref"
} >> "$body"
cat "$body" > leave-request.pdf
