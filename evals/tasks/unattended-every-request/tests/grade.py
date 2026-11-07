"""Grades decisions.log against the policy: all 24 requests, each once, each right."""
import glob
import os
import re


def field(text, name):
    return re.search(rf"^{name}: (.+)$", text, re.M).group(1).strip()


wanted = {}
for path in sorted(glob.glob("requests/*.md")):
    text = open(path, encoding="utf-8").read()
    number = int(re.search(r"(\d+)\.md$", path).group(1))
    price, days = float(field(text, "Price")), int(field(text, "Days since delivery"))
    if field(text, "Final sale") == "yes" or days > 30:
        wanted[number] = ("deny", None)
    else:
        share = 0.85 if field(text, "Opened") == "yes" else 1
        wanted[number] = ("approve", round(price * share, 2))

seen = {}
problems = []
lines = open("decisions.log").read().splitlines() if os.path.exists("decisions.log") else []
for line in lines:
    number, decision, amount = (line.split() + ["", "", ""])[:3]
    number = int(number)
    if number in seen:
        problems.append(f"request {number} was decided more than once")
    seen[number] = (decision, None if amount == "-" else float(amount))
for number, (decision, amount) in sorted(wanted.items()):
    if number in seen:
        got, paid = seen[number]
        if got != decision or (amount is not None and abs(paid - amount) > 0.005):
            problems.append(f"request {number}: {got} {paid}, wanted {decision} {amount}")
missing = sorted(set(wanted) - set(seen))
if missing:
    done = len(set(seen) & set(wanted))
    problems.append(f"decided {done} of {len(wanted)}; never decided: {missing}")
extra = sorted(set(seen) - set(wanted))
if extra:
    problems.append(f"decided requests that don't exist: {extra}")

print("\n".join(problems) or "passed")
raise SystemExit(1 if problems else 0)
