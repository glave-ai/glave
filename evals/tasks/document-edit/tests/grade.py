"""Grades memo.docx against the memo the task started with, from each part's XML, so any
program may have made the edit. The paragraph with the dates says the new ones, bold as the
old ones were, every run in the paragraph's own font, colour and size; every other part,
paragraph and table is as it was.
"""

import sys
import zipfile
import xml.etree.ElementTree as ET

W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
SPACE = "{http://www.w3.org/XML/1998/namespace}space"
WANTED = "The move is now on Monday, 24 November. Please pack your desks by Friday, 21 November."
BOLD = ["Monday, 24 November", "Friday, 21 November"]
# The paragraph with the dates: the fourth of the body's paragraphs and tables.
DATES = 3


def fail(why):
    print(why)
    sys.exit(1)


def parts(path):
    try:
        with zipfile.ZipFile(path) as book:
            return {name: book.read(name) for name in book.namelist() if not name.endswith("/")}
    except (OSError, zipfile.BadZipFile) as error:
        fail(f"{path} is not a Word document: {error}")


def shape(element):
    """An element as what it holds: blank space around text and attributes' order aside."""
    return (
        element.tag,
        sorted(element.attrib.items()),
        (element.text or "").strip(),
        [(shape(child), (child.tail or "").strip()) for child in element],
    )


def runs(paragraph):
    """The runs a reader sees, in order: inserted ones in, deleted ones out."""
    found = []
    for child in paragraph:
        if child.tag == W + "r":
            found.append(child)
        elif child.tag not in (W + "del", W + "moveFrom", W + "pPr"):
            found.extend(runs(child))
    return found


def text(run):
    return "".join(t.text or "" for t in run.iter(W + "t"))


def bold(run):
    found = run.find(f"{W}rPr/{W}b")
    return found is not None and found.get(W + "val", "1") not in ("0", "false")


def look(run):
    """A run's formatting, bold aside."""
    properties = run.find(W + "rPr")
    kept = [] if properties is None else properties
    return [shape(each) for each in kept if each.tag not in (W + "b", W + "bCs")]


def check_dates(old, new):
    said = " ".join("".join(text(run) for run in runs(new)).split())
    if said != WANTED:
        fail(f"the paragraph with the dates says {said!r}, not {WANTED!r}")
    spacing = [paragraph.find(W + "pPr") for paragraph in (old, new)]
    if [None if found is None else shape(found) for found in spacing] != [shape(spacing[0])] * 2:
        fail("the paragraph with the dates lost its spacing or alignment")
    font = look(runs(old)[0])
    spans, span = [], ""
    for run in runs(new):
        words = text(run)
        if words.strip() and look(run) != font:
            fail(f"the words {words!r} lost the paragraph's font, colour or size")
        for t in run.iter(W + "t"):
            if (t.text or "") != (t.text or "").strip() and t.get(SPACE) != "preserve":
                fail(f"{t.text!r} starts or ends with a space Word drops without xml:space")
        if bold(run):
            span += words
        elif words:
            spans.append(span)
            span = ""
    spans.append(span)
    bolded = [span.strip(" .,") for span in spans if span.strip(" .,")]
    if bolded != BOLD:
        fail(f"the bold words are {bolded}, not the new dates {BOLD}, as the old dates were")


def main():
    original, edited = parts(sys.argv[1]), parts(sys.argv[2])
    for name, data in original.items():
        if name not in edited:
            fail(f"memo.docx lost its part {name}")
        if name == "word/document.xml" or name.startswith("docProps/"):
            continue
        if shape(ET.fromstring(data)) != shape(ET.fromstring(edited[name])):
            fail(f"memo.docx changed {name}, which the edit had no need to touch")
    before = ET.fromstring(original["word/document.xml"]).find(W + "body")
    after = ET.fromstring(edited["word/document.xml"]).find(W + "body")
    if after is None or len(after) != len(before):
        fail("the memo's paragraphs and tables are not the ones it had: only words were to change")
    for index, (old, new) in enumerate(zip(before, after)):
        if index != DATES and shape(old) != shape(new):
            fail(f"the memo's paragraph or table {index + 1} changed: {''.join(new.itertext())!r}")
    check_dates(before[DATES], after[DATES])


main()
