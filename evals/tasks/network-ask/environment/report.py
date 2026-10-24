"""Writes report.txt: the downloads this month, as a person would say them."""

import glave_fixture_intword

DOWNLOADS = 1_234_567

with open("report.txt", "w", encoding="utf-8") as report:
    report.write(f"Downloads this month: {glave_fixture_intword.intword(DOWNLOADS)}\n")

print(open("report.txt", encoding="utf-8").read(), end="")
