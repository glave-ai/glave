"""Grades budget.xlsx against the workbook the task started with, from each part's XML, so any
program may have made the fix. Travel's year total, F4, now adds up all four quarters; the
workbook opens showing right totals, its stored results right or an app told to work every
formula out again; every other cell, its formatting, the chart and every other part are as
they were, but for a chart's own copy of a corrected total, which may hold the right value.
"""

import re
import sys
import zipfile
import xml.etree.ElementTree as ET

MAIN = "{http://schemas.openxmlformats.org/spreadsheetml/2006/main}"
SHEET = "xl/worksheets/sheet1.xml"
BOOK = "xl/workbook.xml"
# The broken cell and the total over it, with the values they should come to.
RIGHT = {"F4": "19800", "F7": "426800"}


def fail(why):
    print(why)
    sys.exit(1)


def parts(path):
    try:
        with zipfile.ZipFile(path) as book:
            return {name: book.read(name) for name in book.namelist() if not name.endswith("/")}
    except (OSError, zipfile.BadZipFile) as error:
        fail(f"{path} is not a workbook: {error}")


def shape(element, leave=()):
    """An element as what it holds, blank space and attributes' order aside, and without the
    children named in `leave`."""
    return (
        element.tag,
        sorted(element.attrib.items()),
        (element.text or "").strip(),
        [shape(child) for child in element if child.tag not in leave],
    )


def cells(sheet):
    return {cell.get("r"): cell for cell in sheet.iter(MAIN + "c")}


def formula(cell):
    found = cell.find(MAIN + "f")
    if found is None:
        return None
    return re.sub(r"[\s$]", "", found.text or "").upper().lstrip("=")


def value(cell):
    found = cell.find(MAIN + "v")
    return None if found is None else (found.text or "").strip()


def adds_every_quarter(fixed):
    if fixed in ("SUM(B4:E4)", "SUM(B4,C4,D4,E4)"):
        return True
    return sorted(fixed.split("+")) == ["B4", "C4", "D4", "E4"]


def number(text):
    try:
        return float(text)
    except (TypeError, ValueError):
        return None


def check_cells(before, after, recomputed):
    if sorted(before) != sorted(after):
        fail("the Budget sheet's cells are not the ones it had: only one formula was to change")
    stale = []
    for name, old in before.items():
        new = after[name]
        if (new.get("s"), new.get("t")) != (old.get("s"), old.get("t")):
            fail(f"{name} lost its formatting")
        if name == "F4":
            if not adds_every_quarter(formula(new) or ""):
                fail(f"F4 is ={formula(new)}, which still doesn't add up all four quarters")
        elif formula(new) != formula(old):
            fail(f"{name}'s formula changed from ={formula(old)} to ={formula(new)}")
        if formula(old) is None:
            if value(new) != value(old):
                fail(f"{name} changed from {value(old)} to {value(new)}")
            continue
        right = RIGHT.get(name, value(old))
        if value(new) is None or number(value(new)) != number(right):
            stale.append(name)
    if "F4" in stale and value(after["F4"]) is not None:
        fail(f"F4 still shows {value(after['F4'])}, the total of the broken formula")
    if stale and not recomputed:
        fail(
            f"{', '.join(stale)} would open showing no value or an old one: store their right "
            "values, or have the app work every formula out again on opening (fullCalcOnLoad)"
        )


def as_before(name, data, stale):
    """A chart's part with the right totals it now caches put back to what it cached before,
    since a chart keeps its own copy of each value it draws; any other part as it is."""
    if not name.startswith("xl/charts/"):
        return data
    for right, old in stale.items():
        data = data.replace(f"<c:v>{right}</c:v>".encode(), f"<c:v>{old}</c:v>".encode())
    return data


def main():
    original, edited = parts(sys.argv[1]), parts(sys.argv[2])
    before, after = ET.fromstring(original[SHEET]), ET.fromstring(edited[SHEET])
    # Each corrected total's right value, by the value the workbook cached before the fix.
    stale = {RIGHT[cell]: value(cells(before)[cell]) for cell in RIGHT}
    for name, data in original.items():
        if name not in edited:
            fail(f"budget.xlsx lost its part {name}")
        if name in (SHEET, BOOK) or name.startswith("docProps/"):
            continue
        if shape(ET.fromstring(data)) != shape(ET.fromstring(as_before(name, edited[name], stale))):
            fail(f"budget.xlsx changed {name}, which the fix had no need to touch")
    book = ET.fromstring(edited[BOOK])
    if shape(ET.fromstring(original[BOOK]), [MAIN + "calcPr"]) != shape(book, [MAIN + "calcPr"]):
        fail("budget.xlsx changed its sheets or its defined name")
    calc = book.find(MAIN + "calcPr")
    recomputed = calc is not None and calc.get("fullCalcOnLoad") in ("1", "true")
    if shape(before, [MAIN + "sheetData"]) != shape(after, [MAIN + "sheetData"]):
        fail("the Budget sheet lost its column widths, frozen header, highlight or chart")
    check_cells(cells(before), cells(after), recomputed)


main()
