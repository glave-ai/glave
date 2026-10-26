---
name: spreadsheets
description: Read, analyze and make spreadsheets (.xlsx, .xlsm, .xls, .ods) with glave's own commands, nothing to install. Use whenever a task reads a workbook, answers questions from one, or asks for a new one with formulas, formatting or charts.
---
# Spreadsheets

glave reads and writes workbooks itself: run `glave doc` with bash. Nothing needs installing,
so don't reach for Python, LibreOffice or a package for this.

## Read a workbook

```sh
glave doc read sales.xlsx                          # every sheet, first 100 rows of each
glave doc read sales.xlsx --sheet Summary --rows 101-300
glave doc csv sales.xlsx --sheet Sales > sales.csv # one sheet's values, for awk, sort or a script
```

`read` lists the sheets and defined names, then each sheet as a table with its row numbers and
column letters, so you can name any cell (`E2`). A formula shows beside the value it last
computed to: `=C2*D2 → 1950`. `→ not computed yet` means the file was written by a program and
no spreadsheet app has opened it since; work the value out yourself if you need it. Values are
the cells' own (`19.5`), not as the sheet formats them (`$19.50`); dates read `2026-01-31`.

Read before you answer: check which rows are headers and totals, and whether a total row is
already there before adding one.

## Make a workbook

Describe it in JSON and write it in one step:

```sh
glave doc write report.xlsx - <<'EOF'
{
  "sheets": [
    {
      "name": "Sales",
      "header": true,
      "rows": [
        ["Month", "Units", "Price", "Revenue"],
        ["Jan", 120, 19.5, "=B2*C2"],
        ["Feb", 135, 19.5, "=B3*C3"],
        ["Total", "=SUM(B2:B3)", null, "=SUM(D2:D3)"]
      ],
      "formats": {"C2:D4": "$#,##0.00"},
      "bold": ["A4:D4"],
      "charts": [
        {"type": "column", "title": "Revenue by month", "categories": "A2:A3",
         "values": ["D2:D3"], "at": "F2"}
      ]
    }
  ],
  "names": {"Revenue": "Sales!$D$2:$D$3"}
}
EOF
```

- **Cells:** numbers, text, `true`/`false`, `null` for an empty cell, a formula as text starting
  with `=`, a date as `{"date": "2026-01-31"}`, and text that starts with `=` as
  `{"text": "=not a formula"}`. Write numbers as numbers, never as text.
- **A sheet** takes `name` and `rows`, and may take `start` (the first row's first cell, `A1`
  when left out), `header` (the first row bold, shaded and frozen, with filters), `formats`
  (number formats by range: `"0.0%"`, `"#,##0"`, `"yyyy-mm-dd"`), `bold` (ranges), `widths`
  (`{"A": 18}`; other columns fit what they hold), `freeze` (the first cell that scrolls, `B2`)
  and `charts`.
- **A chart** takes `type` (`column`, `bar`, `line`, `pie`, `doughnut`, `area` or `scatter`), `at`
  (its top left cell), `values` (one range per series) and may take `categories`, `names` (one
  per series), `title`, `x_title` and `y_title`. A range on another sheet names it: `Sales!B2:B13`.
- **Formulas** use Excel's names and commas: `=SUMIF(Sales!B2:B25,"North",Sales!E2:E25)`. Prefer
  formulas over numbers you worked out, so the workbook stays right when its data changes. A
  formula's value is worked out when the workbook is opened, so `glave doc read` shows it as
  not computed yet.

Read the file back with `glave doc read` to check it holds what you meant.

## Change a workbook

`glave doc write` makes a new workbook; it does not edit one in place. To change one, read
it, write the changed workbook as a new file, and tell the person what you could not carry
over (formatting, charts, comments).
