"""Grades report.xlsx from the workbook's own XML, so any program may have written it.

Orders holds every order under a header row, each amount a number. Summary names each region
beside (or above) a formula over Orders whose ranges cover every order, and a value an app
computed, if there is one, is the region's total. A column chart draws Summary's totals.
"""

import csv
import posixpath
import re
import sys
import zipfile
import xml.etree.ElementTree as ET

MAIN = "{http://schemas.openxmlformats.org/spreadsheetml/2006/main}"
RID = "{http://schemas.openxmlformats.org/officeDocument/2006/relationships}id"
CHART = "{http://schemas.openxmlformats.org/drawingml/2006/chart}"


def fail(why):
    print(why)
    sys.exit(1)


def text_of(element):
    return "".join(t.text or "" for t in element.iter(MAIN + "t"))


def sheet_cells(book, strings):
    """Each sheet's cells by name: {(row, column letters): (kind, value, formula)}."""
    rels = ET.fromstring(book.read("xl/_rels/workbook.xml.rels"))
    targets = {rel.get("Id"): rel.get("Target", "") for rel in rels}
    sheets = {}
    for sheet in ET.fromstring(book.read("xl/workbook.xml")).iter(MAIN + "sheet"):
        target = targets.get(sheet.get(RID), "")
        path = target[1:] if target.startswith("/") else posixpath.join("xl", target)
        cells = {}
        for cell in ET.fromstring(book.read(posixpath.normpath(path))).iter(MAIN + "c"):
            ref = re.fullmatch(r"([A-Z]+)([0-9]+)", cell.get("r", ""))
            kind, value = cell.get("t", "n"), cell.findtext(MAIN + "v")
            if kind == "s" and value is not None:
                value = strings[int(value)]
            elif kind == "inlineStr":
                value = text_of(cell)
            if ref:
                cells[(int(ref.group(2)), ref.group(1))] = (kind, value, cell.findtext(MAIN + "f"))
        sheets[sheet.get("name")] = cells
    return sheets


def check_orders(cells, amounts):
    header = {(v or "").strip().lower(): c for (r, c), (_, v, _) in cells.items() if r == 1}
    for head in ("order", "date", "region", "customer", "amount"):
        if head not in header:
            fail(f"Orders' header row (row 1) has no {head} column")
    seen = {}
    for (row, column), (_, order, _) in cells.items():
        # Each order's row; a row of totals under them is no order's.
        if column != header["order"] or not re.fullmatch(r"[0-9]+(\.0*)?", order or ""):
            continue
        kind, amount, _ = cells.get((row, header["amount"]), (None, None, None))
        if kind != "n" or amount is None:
            fail(f"the amount in Orders row {row} is not a number")
        seen[str(int(float(order)))] = float(amount)
    for order, amount in amounts.items():
        if order not in seen:
            fail(f"Orders has no row for order {order}")
        if abs(seen[order] - amount) > 0.005:
            fail(f"order {order}'s amount is {seen[order]}, not {amount}")


def over_orders(found):
    return [(v, f) for _, v, f in found if f and re.search(r"orders'?!", f, re.I)]


def check_summary(cells, totals, last):
    for region, total in totals.items():
        named = [at for at, (_, v, _) in cells.items() if (v or "").strip() == region]
        if not named:
            fail(f"Summary does not name {region}")
        row, column = named[0]
        beside = [cell for (r, _), cell in cells.items() if r == row]
        formulas = over_orders(beside) or over_orders([cells.get((row + 1, column), (0, 0, 0))])
        if not formulas:
            fail(f"Summary has no formula over the Orders sheet beside or under {region}")
        value, formula = formulas[0]
        ranges = re.findall(r"orders'?!\$?[A-Z]+\$?([0-9]*):\$?[A-Z]+\$?([0-9]*)", formula, re.I)
        if any(a and b and (int(a) > 2 or int(b) < last) for a, b in ranges):
            fail(f"Summary's {region} formula, ={formula}, leaves out some orders")
        # A value only where an app computed one: a program's file has none yet.
        if value and not (re.fullmatch(r"-?[0-9.]+", value) and abs(float(value) - total) < 0.005):
            fail(f"Summary's {region} total is {value}, not {total:.2f}")


def check_chart(book):
    for name in book.namelist():
        if not re.fullmatch(r"xl/charts/chart[0-9]*\.xml", name):
            continue
        for bars in ET.fromstring(book.read(name)).iter(CHART + "barChart"):
            column = bars.find(CHART + "barDir")
            if column is not None and column.get("val") == "col":
                refs = [f.text or "" for v in bars.iter(CHART + "val") for f in v.iter(CHART + "f")]
                if any("summary" in ref.lower() for ref in refs):
                    return
    fail("report.xlsx has no column chart of the Summary sheet's totals")


def main():
    with open(sys.argv[1], newline="") as file:
        orders = list(csv.DictReader(file))
    amounts = {row["order"]: float(row["amount"]) for row in orders}
    totals = {}
    for row in orders:
        totals[row["region"]] = totals.get(row["region"], 0.0) + float(row["amount"])
    try:
        book = zipfile.ZipFile("report.xlsx")
    except (OSError, zipfile.BadZipFile) as error:
        fail(f"report.xlsx is not a workbook: {error}")
    with book:
        shared = "xl/sharedStrings.xml"
        strings = []
        if shared in book.namelist():
            strings = [text_of(si) for si in ET.fromstring(book.read(shared)).iter(MAIN + "si")]
        sheets = sheet_cells(book, strings)
        for name in ("Orders", "Summary"):
            if name not in sheets:
                fail(f"report.xlsx has no sheet named {name}; it has {', '.join(sheets)}")
        check_orders(sheets["Orders"], amounts)
        check_summary(sheets["Summary"], totals, len(orders) + 1)
        check_chart(book)


main()
