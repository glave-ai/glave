---
name: spreadsheets
description: Read, analyze, make and edit spreadsheets (.xlsx, .xlsm, .xlsb, .xls, .ods) with glave's own commands, nothing to install. Use whenever a task reads a workbook, answers questions from one, changes one, or asks for a new one.
---
# Spreadsheets

glave reads and writes workbooks itself: run `glave doc` with bash. Nothing needs installing,
so don't reach for Python, LibreOffice or a package for this.

```sh
glave doc read sales.xlsx                          # every sheet, first 100 rows of each
glave doc read sales.xlsx --sheet Summary --rows 101-300
glave doc csv sales.xlsx --sheet Sales > sales.csv # one sheet's values, for awk, sort or a script
```

`read` lists the sheets and defined names, then shows each sheet as a table with its row
numbers and column letters, so you can name any cell (`E2`). A formula shows beside the value
it last computed to: `=C2*D2 → 1950`. `→ not computed yet` means a program wrote the file and
no spreadsheet app has opened it since; work the value out yourself if you need it. Values are
the cells' own (`19.5`), not as the sheet formats them (`$19.50`); dates read `2026-01-31`. A
sheet marked hidden is still read; say so if you use it.

For sums, counts and averages over many rows, query the sheet with `glave doc sql` (the
`glave:data` skill), rather than adding numbers up by reading them. Before you answer, check
which rows are headers and totals, and that the cells you use cover what was asked.

## Making a workbook

`glave doc write` makes a new .xlsx from a description in JSON, on standard input or in a file
named after the workbook:

```sh
glave doc write report.xlsx <<'EOF'
{"sheets": [{"name": "Sales", "header": true,
  "rows": [["Month", "Units", "Price", "Revenue"],
           ["Jan", 120, 19.5, "=B2*C2"],
           ["Feb", 135, 19.5, "=B3*C3"],
           ["Total", "=SUM(B2:B3)", null, "=SUM(D2:D3)"]],
  "formats": {"C2:D4": "$#,##0.00"}, "widths": {"A": 12},
  "charts": [{"type": "column", "at": "F2", "title": "Revenue",
              "categories": "A2:A3", "values": ["D2:D3"]}]}]}
EOF
```

- A cell is a number (never in quotes), text, `true`, `null` for none, a formula (`"=..."`, in
  Excel's names and commas) or a date (`{"date": "2026-01-31"}`).
- A sheet has `name` and `rows` from A1, and may have `header` (the first row bold, frozen and
  filtered), `formats` (number formats by range), `widths` (by column), `freeze` (the first
  cell that scrolls, `B2`) and `charts`.
- A chart has `type` (column, bar, line, pie, doughnut, area or scatter), `at` and `values` (a
  range per series), and may have `categories`, `names` (one per series), `title`, `x_title`
  and `y_title`. A range on another sheet names it: `Orders!E2:E25`.

Use formulas for totals and anything worked out from other cells, so the workbook stays right
when its data changes. glave stores no results: the app that opens the file computes them, so
reading it back shows `→ not computed yet`; check the formulas themselves. It won't write over
a file unless you add `--replace`, and remaking a workbook loses what the description lacks.

## Editing a workbook

`write` only makes new workbooks. To change one and keep its formatting, charts and everything
else, edit its XML in place:

```sh
glave doc unpack sales.xlsx sales   # its parts, and which is each sheet
# edit the sheet's XML with edit
glave doc pack sales sales.xlsx --replace
glave doc read sales.xlsx           # check the change
```

A cell is `<c r="E5" s="3"><f>SUM(E2:E4)</f><v>5905</v></c>`: its formula without the `=`,
the value it last computed to, and its formatting `s`, which you keep. To fix a formula,
change its `<f>`, delete that cell's `<v>`, and add `fullCalcOnLoad="1"` to `<calcPr>` in
`xl/workbook.xml`, so the app that opens the file works every formula out again. A cell marked
`t="s"` keeps its text in `xl/sharedStrings.xml`.
