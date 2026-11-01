"""Grades `shop`: `fee` takes and gives dollars, and everything else behaves as it did. The
amounts are chosen so that no fee falls on a rounding tie."""
import importlib
import inspect
import os
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.getcwd())
problems = []


def wanted(name, call, value):
    module, function = name.split(".")
    try:
        got = call(getattr(importlib.import_module(f"shop.{module}"), function))
    except Exception as error:
        got = repr(error)
    if got != value:
        problems.append(f"{name}: got {got!r}, wanted {value!r}")


parameters = None
try:
    parameters = list(inspect.signature(importlib.import_module("shop.pricing").fee).parameters)
except Exception as error:
    problems.append(f"shop.pricing.fee: {error!r}")
if parameters != ["dollars", "rate"]:
    problems.append(f"fee takes {parameters}")
wanted("pricing.fee", lambda f: f(12.34, 0.2), 2.47)
wanted("pricing.fee", lambda f: f(100, 0.05), 5.0)
wanted("orders.total_cents", lambda f: f(1234), 1481)
wanted("orders.total_cents", lambda f: f(9999), 11999)
wanted("orders.vat_line", lambda f: f(1234), 247)
wanted("orders.vat_line", lambda f: f(2), 1)
wanted("refunds.service_kept", lambda f: f(12.34), 0.62)
wanted("refunds.paid_back", lambda f: f(12.34), 11.72)
wanted("shipping.charge_cents", lambda f: f(11), 256)
wanted("shipping.charge_cents", lambda f: f(107), 573)
wanted("gifts.wrapping_cents", lambda f: f(2520), 76)
wanted("bundles.extra_cents", lambda f: f("standard", 1234), 123)
wanted("bundles.extra_cents", lambda f: f("large", 1234), 185)
wanted("invoices.line_totals", lambda f: f([1234, 9999]), [1481, 11999])

print("\n".join(problems) or "passed")
raise SystemExit(1 if problems else 0)
