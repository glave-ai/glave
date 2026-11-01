"""Grades the imports, and that once one import worked the failed way wasn't tried again."""
import json
import os

KEYS = {"accounts": "account_code", "contracts": "contract_ref", "invoices": "invoice_number",
        "products": "sku", "suppliers": "vat_id", "vehicles": "plate"}
problems = []
for name in KEYS:
    try:
        table = json.load(open(f"out/{name}.json", encoding="utf-8"))
    except OSError:
        problems.append(f"{name} was not imported")
        continue
    wanted = [f"{name}-{letter}" for letter in "abc"]
    if sorted(table) != wanted:
        problems.append(f"{name} is keyed by the wrong column: {sorted(table)}")

log = open(".import.log").read().splitlines() if os.path.exists(".import.log") else []
states = [line.split()[0] for line in log]
if "imported" in states and "failed" in states[states.index("imported"):]:
    tried = states[states.index("imported"):].count("failed")
    problems.append(f"the failed way was tried {tried} times after a way through was found")

print("\n".join(problems) or "passed")
raise SystemExit(1 if problems else 0)
