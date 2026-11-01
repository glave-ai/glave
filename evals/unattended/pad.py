#!/usr/bin/env python3
"""Fills a task's files to size, as its setup: long files are what make a small context window
compact, and what they say beside the point is not worth keeping in a repository line by line.

Every line of a workspace file holding `@@filler@@` becomes filler text, with the line's own text
before the marker (`# `, say) as each line's start. A file's markers share the characters the
first argument names, 6000 when it names none. The same path always gets the same text, so an
attempt's files are the same every time."""

import os
import random
import sys
import textwrap

MARKER = "@@filler@@"
SIZE = int(sys.argv[1]) if len(sys.argv) > 1 else 6000

WHEN = ["In the March review", "During the autumn audit", "At last year's planning day",
        "After the warehouse move", "In the second quarter", "Before the supplier change",
        "During the winter freeze", "At the June retrospective", "After the data centre visit"]
WHO = ["the operations group", "two of the support leads", "the finance contact",
       "the on-call engineers", "a visiting consultant", "the regional managers",
       "the accessibility reviewers", "the night shift", "the partner team"]
DID = ["went through", "compared notes on", "wrote up", "spent an afternoon on", "looked again at",
       "collected complaints about", "made a chart of", "argued for a while over"]
WHAT = ["the delivery estimates for the northern routes", "how long the morning reports take",
        "the colour of the status badges", "which printers still jam on the thick labels",
        "the old spreadsheet nobody admits to owning", "the wording of the renewal letters",
        "parking at the east entrance", "the backlog of address corrections",
        "how often the dashboard is opened on a phone", "the seating plan for the third floor"]
THEN = ["Nothing came of it.", "The notes were filed and nobody has asked for them since.",
        "It was left as it was.", "A follow-up was planned and quietly dropped.",
        "Everyone agreed it could wait.", "It made no difference to anything here.",
        "The conclusion was that it did not matter much.", "The minutes say little more."]


def filler(seed, size):
    """About `size` characters of history that bears on nothing, the same for the same seed."""
    pick = random.Random(seed).choice
    text = ""
    while len(text) < size:
        text += f"{pick(WHEN)} {pick(WHO)} {pick(DID)} {pick(WHAT)}. {pick(THEN)} "
    return text


def padded(path, text):
    """`text`, the file at `path`, with its markers filled."""
    lines = text.splitlines(keepends=True)
    share = SIZE // max(sum(MARKER in line for line in lines), 1)
    out = []
    for number, line in enumerate(lines):
        if MARKER in line:
            start = line.split(MARKER)[0]
            body = textwrap.fill(filler(f"{path}:{number}", share), 96, initial_indent=start,
                                 subsequent_indent=start)
            out.append(body + "\n")
        else:
            out.append(line)
    return "".join(out)


def main():
    for folder, _, names in os.walk("."):
        for name in names:
            path = os.path.join(folder, name)
            with open(path, encoding="utf-8", errors="surrogateescape") as file:
                text = file.read()
            if MARKER in text:
                with open(path, "w", encoding="utf-8", errors="surrogateescape") as file:
                    file.write(padded(path, text))


main()
