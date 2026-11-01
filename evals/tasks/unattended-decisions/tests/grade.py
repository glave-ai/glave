"""Grades settings.py, and that the tickets were closed once each, in order."""
import os

settings = {}
exec(open("settings.py", encoding="utf-8").read(), settings)
values = {name: value for name, value in settings.items() if not name.startswith("__")}

# Names the tickets give, exactly; the rest only by the ending a decision gives them, and their
# value: a later ticket names no setting, so any fitting name is right.
NAMED = {
    "connect_timeout_ms": 5000,
    "ff_beta_search": False,
    "upload_limit_bytes": 10485760,
}
BY_ENDING = [
    ("read timeout", "_ms", 30000),
    ("dark mode flag", "ff_", True),
    ("cache size", "_bytes", 2097152),
    ("cache lifetime", "_ms", 600000),
]
problems = []
for name, value in NAMED.items():
    if name not in values:
        problems.append(f"no {name}")
    elif values[name] != value or type(values[name]) is not type(value):
        problems.append(f"{name} is {values[name]!r}, wanted {value!r}")
taken = set(NAMED)
for what, ending, value in BY_ENDING:
    fits = [name for name, held in values.items() if name not in taken
            and (name.startswith(ending) if ending.endswith("_") else name.endswith(ending))
            and held == value and type(held) is type(value)]
    if fits:
        taken.add(fits[0])
    else:
        problems.append(f"no {what}: a setting ending {ending} holding {value!r}")

regions = {"eu-west-1", "us-east-1"}
default, backup = values.get("default_region"), values.get("backup_region")
if default not in regions or backup not in regions or default == backup:
    problems.append(f"the regions are {default!r} and {backup!r}")

allowed = taken | {"default_region", "backup_region"}
extra = sorted(set(values) - allowed)
if extra:
    problems.append(f"settings that should be gone or have other names: {extra}")

closed = open("closed.log").read().split() if os.path.exists("closed.log") else []
wanted = list(range(1, 9))
if [int(number) if number.isdigit() else number for number in closed] != wanted:
    problems.append(f"closed {closed}, wanted each once, in order: {wanted}")

print("\n".join(problems) or "passed")
raise SystemExit(1 if problems else 0)
