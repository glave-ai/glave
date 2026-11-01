"""Grades settings.py, and that the tickets were closed once each, in order."""
import os

settings = {}
exec(open("settings.py", encoding="utf-8").read(), settings)
values = {name: value for name, value in settings.items() if not name.startswith("__")}

WANTED = {
    "connect_timeout_ms": 5000,
    "ff_beta_search": False,
    "upload_limit_bytes": 10485760,
    "read_timeout_ms": 30000,
    "ff_dark_mode": True,
    "cache_size_bytes": 2097152,
    "cache_lifetime_ms": 600000,
}
problems = []
for name, value in WANTED.items():
    if name not in values:
        problems.append(f"no {name}")
    elif values[name] != value or type(values[name]) is not type(value):
        problems.append(f"{name} is {values[name]!r}, wanted {value!r}")

regions = {"eu-west-1", "us-east-1"}
default, backup = values.get("default_region"), values.get("backup_region")
if default not in regions or backup not in regions or default == backup:
    problems.append(f"the regions are {default!r} and {backup!r}")

allowed = set(WANTED) | {"default_region", "backup_region"}
extra = sorted(set(values) - allowed)
if extra:
    problems.append(f"settings that should be gone or have other names: {extra}")

closed = open("closed.log").read().split() if os.path.exists("closed.log") else []
wanted = list(range(1, 9))
if [int(number) if number.isdigit() else number for number in closed] != wanted:
    problems.append(f"closed {closed}, wanted each once, in order: {wanted}")

print("\n".join(problems) or "passed")
raise SystemExit(1 if problems else 0)
