"""Grades leave-request-filled.pdf against the form the task started with, from the PDF's own
objects, so any program may have filled it. The person's fields hold what they said, in the
form's date format; the radio button and checkbox show as chosen; the manager's part stays
empty; the form keeps every field, and its page says what it said, so nothing was drawn over
it; and a viewer shows the values: the form asks viewers to draw them (NeedAppearances), or
each filled field's look shows its value.
"""

import re
import sys
import zlib

WANTED = {
    "Text1": r"sam okafor",
    "Text2": r"support( team)?",
    "Text3": r"0?1/12/2026",
    "Text4": r"0?5/12/2026",
    "Text5": r"5",
}
ON = {"Group1": "Annual", "Check Box1": "Yes"}
MANAGERS = ["Text6", "Check Box2"]
LABELS = {
    "Text1": "Employee name", "Text2": "Team", "Text3": "First day off",
    "Text4": "Last day off", "Text5": "Working days", "Group1": "Type of leave",
    "Check Box1": "My work is handed over", "Text6": "the manager's name",
    "Check Box2": "Approved",
}

DELIMITERS = b"()<>[]{}/%"
SPACE = b" \t\r\n\f\0"


def fail(why):
    print(why)
    sys.exit(1)


class Name(str):
    pass


class Ref(int):
    pass


class Stream:
    def __init__(self, entries, raw):
        self.entries, self.raw = entries, raw

    def data(self):
        """The stream's bytes decoded, or None when a filter isn't Flate."""
        filters = self.entries.get("Filter", [])
        data = self.raw
        for each in filters if isinstance(filters, list) else [filters]:
            if each != "FlateDecode":
                return None
            data = zlib.decompress(data)
        return data


def skip(data, at):
    while at < len(data):
        if data[at] in SPACE:
            at += 1
        elif data[at : at + 1] == b"%":
            while at < len(data) and data[at] not in b"\r\n":
                at += 1
        else:
            break
    return at


def literal(data, at):
    out, depth, at = bytearray(), 1, at + 1
    escapes = {b"n": b"\n", b"r": b"\r", b"t": b"\t", b"b": b"\b", b"f": b"\f"}
    while depth and at < len(data):
        c = data[at : at + 1]
        if c == b"\\":
            octal = re.match(rb"[0-7]{1,3}", data[at + 1 : at + 4])
            if octal:
                out.append(int(octal.group(), 8) & 255)
                at += 1 + len(octal.group())
                continue
            following = data[at + 1 : at + 2]
            out += escapes.get(following, b"" if following in b"\r\n" else following)
            at += 2
            continue
        depth += {b"(": 1, b")": -1}.get(c, 0)
        if depth:
            out += c
        at += 1
    return bytes(out), at


def value(data, at):
    """The object at `at` in `data`, and where it ends."""
    at = skip(data, at)
    for opening, closing in ((b"<<", b">>"), (b"[", b"]")):
        if data.startswith(opening, at):
            items, at = [], at + len(opening)
            while at < len(data) and not data.startswith(closing, skip(data, at)):
                item, at = value(data, at)
                items.append(item)
            at = skip(data, at) + len(closing)
            if opening == b"[":
                return items, at
            return dict(zip(items[::2], items[1::2])), at
    if data.startswith(b"(", at):
        return literal(data, at)
    if data.startswith(b"<", at):
        end = data.index(b">", at)
        digits = re.sub(rb"\s", b"", data[at + 1 : end])
        return bytes.fromhex((digits + b"0" * (len(digits) % 2)).decode()), end + 1
    end = at + 1
    while end < len(data) and data[end] not in SPACE + DELIMITERS:
        end += 1
    word = data[at:end]
    if word.startswith(b"/"):
        name = re.sub(rb"#([0-9A-Fa-f]{2})", lambda m: bytes([int(m.group(1), 16)]), word[1:])
        return Name(name.decode("latin-1")), end
    reference = re.match(rb"\s+(\d+)\s+R\b", data[end : end + 20])
    if word.isdigit() and reference:
        return Ref(int(word)), end + reference.end()
    if word in (b"true", b"false"):
        return word == b"true", end
    try:
        return (float(word) if b"." in word else int(word)), end
    except ValueError:
        return None, end


def objects(data):
    """Each object by number, a later one over an earlier as updates are added at the end,
    those packed in object streams among them."""
    found, at = {}, 0
    header = re.compile(rb"(\d+)\s+\d+\s+obj\b")
    while match := header.search(data, at):
        number, (thing, at) = int(match.group(1)), value(data, match.end())
        start = skip(data, at)
        if isinstance(thing, dict) and data.startswith(b"stream", start):
            start = data.index(b"\n", start) + 1
            length = thing.get("Length")
            end = start + length if type(length) is int else None
            if end is None or not data.startswith(b"endstream", skip(data, end)):
                end = data.index(b"endstream", start)
            thing, at = Stream(thing, data[start:end]), end
        found[number] = thing
        if isinstance(thing, Stream) and thing.entries.get("Type") == "ObjStm":
            packed, first = thing.data() or b"", thing.entries.get("First", 0)
            pairs = packed[:first].split()
            for index in range(0, len(pairs) - 1, 2):
                found[int(pairs[index])] = value(packed, first + int(pairs[index + 1]))[0]
    return found


def text(raw):
    if isinstance(raw, bytes):
        if raw.startswith(b"\xfe\xff"):
            return raw[2:].decode("utf-16-be", "replace")
        utf8 = raw.startswith(b"\xef\xbb\xbf")
        return raw.decode("utf-8" if utf8 else "latin-1").lstrip("﻿")
    return "" if raw is None else str(raw)


def form(path):
    """How many pages a PDF has and what each draws, its form's own entries, and each field by
    its full name: its entries, with what it took from above, and its widgets."""
    try:
        data = open(path, "rb").read()
    except OSError as error:
        fail(f"no {path}: {error}")
    if b"/Encrypt" in data:
        fail(f"{path} is encrypted")
    found = objects(data)
    get = lambda thing: found.get(thing) if isinstance(thing, Ref) else thing
    kids = lambda node: [k for k in map(get, get(node.get("Kids")) or []) if isinstance(k, dict)]
    catalogs = [o for o in found.values() if isinstance(o, dict) and o.get("Type") == "Catalog"]
    if not catalogs:
        fail(f"{path} is not a PDF this grader can read")
    pages, nodes = [], [get(catalogs[-1].get("Pages")) or {}]
    while nodes:
        below = kids(nodes.pop(0))
        pages.extend(kid for kid in below if kid.get("Type") == "Page")
        nodes[:0] = [kid for kid in below if kid.get("Type") != "Page"]
    drawn = []
    for page in pages:
        parts = get(page.get("Contents"))
        for part in map(get, parts if isinstance(parts, list) else [parts]):
            content = (part.data() or part.raw) if isinstance(part, Stream) else b""
            drawn.append(b" ".join(content.split()))
    acro = get(catalogs[-1].get("AcroForm")) or {}
    fields, waiting = {}, [(field, "", {}) for field in kids({"Kids": acro.get("Fields")})]
    while waiting:
        field, above, inherited = waiting.pop(0)
        entries = dict(inherited, **{k: field[k] for k in ("FT", "Ff", "V") if k in field})
        own = text(field.get("T"))
        name = f"{above}.{own}" if above and own else own or above
        named = [kid for kid in kids(field) if "T" in kid]
        waiting.extend((kid, name, entries) for kid in named)
        if not named:
            fields[name] = (dict(field, **entries), kids(field) or [field])
    return (len(pages), drawn), acro, fields, get


def main():
    page, _, before, _ = form(sys.argv[1])
    pages, acro, fields, get = form(sys.argv[2])
    if pages[0] != page[0]:
        fail(f"the filled form has {pages[0]} pages, not the form's {page[0]}")
    if pages != page:
        fail("the page's own text and lines changed: fill the form's fields, don't draw on it")
    lost = sorted(set(before) - set(fields))
    if lost:
        fail(f"the filled form lost the fields {lost}: they should stay, filled in")
    looks = lambda widget: get((get(widget.get("AP")) or {}).get("N"))
    redrawn = acro.get("NeedAppearances") is True
    for name, wanted in WANTED.items():
        entries, widgets = fields[name]
        said = " ".join(text(entries.get("V")).split())
        if not re.fullmatch(wanted, said, re.I):
            fail(f"{name} ({LABELS[name]}) holds {said!r}")
        for look in map(looks, widgets):
            shown = (look.data() if isinstance(look, Stream) else None) or b""
            if not redrawn and said.encode("latin-1", "replace") not in shown:
                fail(f"{name} holds {said!r}, but a viewer would show its old look")
    for name, state in ON.items():
        entries, widgets = fields[name]
        if entries.get("V") != state:
            fail(f"{name} ({LABELS[name]}) is {text(entries.get('V'))!r}, not {state!r}")
        states = lambda widget: looks(widget) if isinstance(looks(widget), dict) else {}
        shows = [widget.get("AS") for widget in widgets if state in states(widget)]
        if shows != [state]:
            fail(f"{name}'s {state} box doesn't show as chosen: {shows}")
    for name in MANAGERS:
        entries, widgets = fields[name]
        held = text(entries.get("V")).strip()
        if held not in ("", "Off") or any(w.get("AS") not in (None, "Off") for w in widgets):
            fail(f"{name} ({LABELS[name]}) is the manager's to fill, and holds {held!r}")


main()
