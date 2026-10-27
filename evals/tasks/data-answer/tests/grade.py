"""Grades answers.json against orders.csv as the task made it, worked out here with Python's
standard library: each answer exact, a number within a cent (a correlation within 0.001), and
each name in any case.
"""

import csv
import json
import statistics
import sys

orders_csv, answers_json = sys.argv[1], sys.argv[2]

with open(orders_csv, newline="") as f:
    orders = list(csv.DictReader(f))


def net(order):
    return int(order["units"]) * float(order["unit_price"]) * (1 - float(order["discount"]))


kept = [o for o in orders if o["returned"] == "no"]


def most(groups, measure):
    measured = {name: measure(values) for name, values in groups.items()}
    top = max(measured, key=measured.get)
    return top, measured[top]


def by(key):
    groups = {}
    for o in kept:
        groups.setdefault(o[key], []).append(net(o))
    return groups


region, median = most(by("region"), statistics.median)
category, spread = most(by("category"), statistics.stdev)
wanted = {
    "q2_net_revenue": (sum(net(o) for o in kept if "2026-04" <= o["date"][:7] <= "2026-06"), 0.01),
    "top_region": region,
    "top_region_median": (median, 0.01),
    "discount_units_corr": (
        statistics.correlation(
            [float(o["discount"]) for o in orders], [float(o["units"]) for o in orders]
        ),
        0.001,
    ),
    "most_variable_category": category,
    "its_stddev": (spread, 0.01),
}

try:
    with open(answers_json) as f:
        answers = json.load(f)
except (OSError, ValueError) as error:
    sys.exit(f"answers.json can't be read as JSON: {error}")
if not isinstance(answers, dict):
    sys.exit("answers.json is not a JSON object")

wrong = []
for key, want in wanted.items():
    got = answers.get(key)
    if isinstance(want, tuple):
        value, tolerance = want
        ok = isinstance(got, (int, float)) and not isinstance(got, bool)
        if not ok and isinstance(got, str):
            try:
                got, ok = float(got.replace(",", "").lstrip("$")), True
            except ValueError:
                pass
        if not ok or abs(got - value) > tolerance + 1e-9:
            wrong.append(f"{key} is {answers.get(key)!r}, not {value:.4f}")
    elif not isinstance(got, str) or got.strip().lower() != want.lower():
        wrong.append(f"{key} is {got!r}, not {want!r}")
if wrong:
    sys.exit("; ".join(wrong))
