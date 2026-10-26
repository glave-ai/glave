---
name: spreadsheets
description: Read and analyze spreadsheets (.xlsx, .xlsm, .xlsb, .xls, .ods) with glave's own commands, nothing to install. Use whenever a task reads a workbook or answers questions from one.
---
# Spreadsheets

glave reads workbooks itself: run `glave doc` with bash. Nothing needs installing, so don't
reach for Python, LibreOffice or a package for this.

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

For sums, counts and averages over many rows, take the sheet as CSV and compute with `awk` or
whatever the machine has, rather than adding numbers up by reading them. Before you answer,
check which rows are headers and totals, and that the cells you use cover what was asked.

glave doesn't make or change workbooks yet. For a new table, write a CSV, which every
spreadsheet app opens, and say that is what you made.
