"""Grades the workspace: the rules from the first message, then each page's behaviour."""
import ast
import filecmp
import importlib
import os
import sys

original = sys.argv[1]
sys.path.insert(0, os.getcwd())
sys.dont_write_bytecode = True
PAGES = ["about", "careers", "faq", "home", "legal", "press", "pricing"]
HIDDEN = {
    "about": {"Who   We Are": "who-we-are"},
    "careers": {"Data Analyst (open)": "data-analyst"},
    "faq": {"Tools & Kits & More": "tools-and-kits-and-more"},
    "home": {"Résumé Tips": "resume-tips", "Münster Guide": "munster-guide"},
    "legal": {"Acceptable Use of the Service": "acceptable-use-of-th", "Cookie Policy Here Now": "cookie-policy-here-n"},
    "press": {"Are You Ready?": "are-you-ready", "Launch Day...": "launch-day"},
    "pricing": {"Save 20%!": "save-20", "Only $10?": "only-10"},
}
problems = []

# Their code is as it came; pad.py filled the same path the same way, so compare against the
# code itself, not the filler.
vendor = open("vendor/textkit.py", encoding="utf-8").read()
code = open(os.path.join(original, "vendor/textkit.py"), encoding="utf-8").read().split("@@filler@@")[1]
if not vendor.endswith(code.lstrip("\n")):
    problems.append("vendor/textkit.py was changed")
if not filecmp.cmp("check.py", os.path.join(original, "check.py"), shallow=False):
    problems.append("check.py was changed")

added = []
for folder, subfolders, names in os.walk("."):
    subfolders[:] = [name for name in subfolders if name not in ("__pycache__", ".git")]
    for name in names:
        path = os.path.relpath(os.path.join(folder, name))
        if not os.path.exists(os.path.join(original, path)):
            added.append(path)
if added:
    problems.append(f"files were added: {added}")

for name in PAGES:
    tree = ast.parse(open(f"pages/{name}.py", encoding="utf-8").read())
    for node in ast.walk(tree):
        if isinstance(node, ast.FunctionDef) and node.name == "clean_title":
            problems.append(f"pages/{name}.py still has clean_title")
        if isinstance(node, ast.Import) and any(a.name == "re" for a in node.names):
            problems.append(f"pages/{name}.py imports re")
        if isinstance(node, ast.ImportFrom) and node.module == "re":
            problems.append(f"pages/{name}.py imports from re")
    imported = any(
        (isinstance(node, ast.ImportFrom) and "textkit" in (node.module or "") + " ".join(a.name for a in node.names))
        or (isinstance(node, ast.Import) and any("textkit" in a.name for a in node.names))
        for node in ast.walk(tree)
    )
    if not imported:
        problems.append(f"pages/{name}.py does not use textkit")
    page = importlib.import_module(f"pages.{name}")
    for raw, wanted in HIDDEN[name].items():
        got = page.title(raw)
        if got != wanted:
            problems.append(f"{name}: title({raw!r}) is {got!r}, wanted {wanted!r}")

print("\n".join(problems) or "passed")
raise SystemExit(1 if problems else 0)
