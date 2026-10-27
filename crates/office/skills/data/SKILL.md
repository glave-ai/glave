---
name: data
description: Answer questions from CSV, JSON and spreadsheet data with SQL, using glave's own SQLite, nothing to install. Use whenever a task sums, counts, averages, ranks, groups, joins or finds statistics in a data file.
---
# Data

`glave doc sql` loads CSV, TSV, JSON (an array of objects) and workbook files as tables in
SQLite and runs a query on them. Nothing needs installing, so don't reach for Python, pandas
or a database for this. Use it over `awk` or `sort | uniq` once a question groups, joins,
filters on more than one column, or needs a median or a deviation: SQL says it in one line,
and gets quoting, headers and empty cells right.

```sh
glave doc sql sales.csv                   # each table: its name, rows, columns and their types
glave doc sql sales.csv regions.json "
  SELECT r.manager, s.region, round(sum(s.units * s.price), 2) AS revenue,
         median(s.units) AS typical_order
  FROM sales s JOIN regions r ON r.region = s.region
  WHERE s.day >= '2026-07-01'
  GROUP BY s.region, r.manager ORDER BY revenue DESC"
```

- A table is named after its file (`sales-2026.csv` is `sales_2026`); `orders=data.csv` names
  it. A workbook of several sheets is a table each, `<file>_<sheet>`, unless `--sheet <name>`
  loads one.
- The first row is the header, its words the columns' names. A name with spaces, or one SQL
  reserves such as `order`, takes double quotes (`"Unit price"`), as the first command shows.
- Columns are INTEGER, REAL or TEXT, from their values. Empty cells are NULL, which `count`,
  `sum` and `avg` leave out. Dates are text, `2026-01-31`, so compare them as text and use
  `strftime('%m', day)`.
- Beyond SQLite's own functions: `sqrt`, `power`, `ln`, `log10`, `floor`, `ceil`, `median`,
  `percentile(x, 90)`, `stddev`, `variance` (a sample's; `stddev_pop` and `var_pop` for a whole
  population) and `corr(x, y)`.
- The answer prints as a table, its first 100 rows; `--csv` prints every row as CSV, to save
  or pipe on. Round money with `round(x, 2)` only at the end.
